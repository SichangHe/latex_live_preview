/** Engine actor: owns the LuaLaTeX generations and their snapshots; see docs/engine.md. */
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const ROOT = path.resolve(import.meta.dirname, "..");

/** Content signature of the aux files (`.aux`, `.bbl`, `.toc`, ...) in `dir`. */
export function seedSig(dir) {
  return fs.readdirSync(dir).filter((f) => !NOT_SEED.has(f)).sort()
    .map((f) => f + "\0" + fs.readFileSync(path.join(dir, f), "latin1")).join("\0");
}
/** Files a run writes that are not inputs of the next run. */
export const NOT_SEED = new Set(["main.pdf", "main.synctex", "main.log"]);
/** Each parked snapshot costs ~5 MB; beyond this many, keep every 2nd, 4th, ... page. */
const MAX_SNAPS = Number(process.env.LLP_MAX_SNAPS ?? 400);
const WATCHDOG_MS = 3000;

// 🧑 "This also needs to work for very long documents. So you need incremental change support."
/** Creates an engine for `mainTex` with scratch state under `runDir`.
 * `onPdf({gen, kind, firstChanged, dir, seed})` fires when a PDF is complete (`partial` or
 * `done`); pages before `firstChanged` are identical to the previous generation's; `seed` is
 * the `seedSig` of the aux files the generation's lineage started from. */
export function createEngine({ mainTex, runDir, onPdf, log }) {
  const shadow = path.join(runDir, "shadow");
  const evPath = path.join(runDir, "ev");
  fs.rmSync(runDir, { recursive: true, force: true });
  fs.mkdirSync(shadow, { recursive: true });
  if (spawnSync("mkfifo", [evPath]).status !== 0) throw new Error("mkfifo failed");
  const evFd = fs.openSync(evPath, "r+");
  /** page -> {pid, maxRead: Map<file, nLines>} */
  const snaps = new Map();
  /** only pages divisible by this keep snapshots */
  let stride = 1;
  let gen = 0;
  /** pages before this one are unchanged in the current generation */
  let firstChanged = 1;
  /** gen -> seedSig of the cold gen it descends from */
  const seedOf = new Map();
  /** pids of processes running the current generation */
  let running = [];
  /** process groups of cold gens; all snapshots, clones and finishers are inside them */
  const groups = new Set();
  /** no PDF of a finished gen was delivered since `firstChanged` was last reset */
  let delivered = true;
  /** pending resume: if its clone does not report, the snapshot is presumed dead */
  let watchdog = null;

  const env = {
    ...process.env,
    LLP_NATIVE: path.join(ROOT, "native/llpfork.so"),
    LLP_EVENTS: evPath,
    LLP_RUN: runDir,
    LLP_SHADOW: shadow,
  };

  function tellSnap(pid, cmd) {
    try {
      const fd = fs.openSync(path.join(runDir, `snap-${pid}`), "r+");
      fs.writeSync(fd, cmd + "\n");
      fs.closeSync(fd);
      if (cmd === "quit") fs.rmSync(path.join(runDir, `snap-${pid}`), { force: true });
    } catch {}
  }

  /** Pages from the returned one on may differ from what the viewer last got complete. */
  function markChanged(page) {
    firstChanged = delivered ? page : Math.min(firstChanged, page);
    delivered = false;
  }

  function killRunning() {
    for (const pid of running) {
      try { process.kill(pid, "SIGKILL"); } catch {}
    }
    running = [];
  }

  function dropSnap(page) {
    tellSnap(snaps.get(page).pid, "quit");
    snaps.delete(page);
  }

  /** Compiles from scratch, starting from the auxiliary files (`.aux`, `.bbl`, ...) in
   * `seedDir` if given. Existing snapshots stay usable until the new run replaces them. */
  function cold(seedDir) {
    killRunning();
    gen += 1;
    markChanged(1);
    if (!seedDir) stride = 1;
    seedOf.set(gen, seedDir ? seedSig(seedDir) : "");
    const dir = path.join(runDir, `g${gen}`);
    fs.mkdirSync(dir);
    for (const f of seedDir ? fs.readdirSync(seedDir) : []) {
      if (!NOT_SEED.has(f)) fs.copyFileSync(path.join(seedDir, f), path.join(dir, f));
    }
    const driver = path.join(ROOT, "engine/driver.lua");
    const tex = `\\directlua{dofile("${driver}")}\\input{${mainTex}}`;
    const p = spawn("bash", ["-c", `exec 200<"$LLP_OUT" && exec lualatex --shell-escape -synctex=-1 -interaction=nonstopmode -file-line-error -jobname=main -output-directory=/proc/self/fd/200 "$0"`, tex], {
      cwd: path.dirname(mainTex),
      env: { ...env, LLP_GEN: String(gen), LLP_OUT: dir },
      stdio: "ignore",
      detached: true,
    });
    groups.add(p.pid);
    running = [p.pid];
    log(`gen ${gen}: cold start`);
  }

  /** Recompiles after `file` changed from 1-based line `line` on: resumes the latest
   * snapshot that has not read that line; snapshots that have are dropped after. */
  function edit(file, line) {
    const stale = [...snaps].filter(([, s]) => (s.maxRead.get(file) ?? 0) >= line).map(([page]) => page);
    const best = Math.max(0, ...[...snaps.keys()].filter((page) => !stale.includes(page)));
    if (!best) {
      for (const page of stale) dropSnap(page);
      return cold(null);
    }
    killRunning();
    gen += 1;
    markChanged(best + 1);
    seedOf.set(gen, seedOf.get(snaps.get(best).gen));
    tellSnap(snaps.get(best).pid, `go ${gen}`);
    clearTimeout(watchdog);
    const g = gen;
    watchdog = setTimeout(() => {
      if (gen !== g) return;
      log(`gen ${g}: snapshot of page ${best} did not resume; cold start`);
      snaps.delete(best);
      cold(null);
    }, WATCHDOG_MS);
    for (const page of stale) dropSnap(page);
    log(`gen ${gen}: resume after page ${best}`);
  }

  /** Mirrors an unsaved editor buffer so TeX reads it instead of the file on disk. */
  function writeShadow(file, text) {
    const p = path.join(shadow, file);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, text);
  }

  function onEvent(lineStr) {
    if (process.env.LLP_DEBUG) log(lineStr.slice(0, 80));
    const [g, kind, ...rest] = lineStr.split(" ");
    if (Number(g) !== gen) {
      if (kind === "snap") tellSnap(rest[1], "quit");
      if (kind === "run") try { process.kill(Number(rest[1]), "SIGKILL"); } catch {}
      return;
    }
    switch (kind) {
      case "snap": {
        const [page, pid, mr = ""] = [Number(rest[0]), rest[1], rest.slice(2).join(" ")];
        if (page % stride) {
          tellSnap(pid, "quit");
          break;
        }
        if (snaps.has(page)) dropSnap(page);
        const maxRead = new Map(mr.split("\t").filter(Boolean).map((kv) => {
          const i = kv.lastIndexOf(":");
          return [path.normalize(kv.slice(0, i)), Number(kv.slice(i + 1))];
        }));
        snaps.set(page, { pid, maxRead, gen });
        if (snaps.size > MAX_SNAPS) {
          stride *= 2;
          for (const pg of [...snaps.keys()]) if (pg % stride) dropSnap(pg);
        }
        break;
      }
      case "run":
        clearTimeout(watchdog);
        running.push(Number(rest[1]));
        break;
      case "partial": onPdf({ gen, kind, firstChanged, dir: rest[1], seed: seedOf.get(gen) }); break;
      case "done":
        onPdf({ gen, kind, firstChanged, dir: rest[0], seed: seedOf.get(gen) });
        delivered = true;
        running = [];
        break;
      case "error": log(`gen ${gen} error: ${rest.join(" ")}`); break;
    }
  }

  readline.createInterface({ input: fs.createReadStream(null, { fd: evFd }) }).on("line", onEvent);

  /** Kills every process this engine started, directly or by forking. */
  function close() {
    clearTimeout(watchdog);
    for (const g of groups) try { process.kill(-g, "SIGKILL"); } catch {}
  }

  function dropShadow(file) {
    fs.rmSync(path.join(shadow, file), { force: true });
  }

  return { cold, edit, writeShadow, dropShadow, close, gen: () => gen };
}

