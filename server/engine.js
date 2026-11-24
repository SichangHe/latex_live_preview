/** Engine actor: owns the LuaLaTeX generations and their snapshots; see docs/engine.md. */
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const ROOT = path.resolve(import.meta.dirname, "..");
/** Files a run writes that are not inputs of the next run. */
export const NOT_SEED = new Set(["main.pdf", "main.synctex", "main.log"]);

/** Creates an engine for `mainTex` with scratch state under `runDir`.
 * `onPdf({gen, kind, firstChanged, dir})` fires when a PDF is complete (`partial` or
 * `done`); pages before `firstChanged` are identical to the previous generation's. */
export function createEngine({ mainTex, runDir, onPdf, log }) {
  const shadow = path.join(runDir, "shadow");
  const evPath = path.join(runDir, "ev");
  fs.rmSync(runDir, { recursive: true, force: true });
  fs.mkdirSync(shadow, { recursive: true });
  if (spawnSync("mkfifo", [evPath]).status !== 0) throw new Error("mkfifo failed");
  const evFd = fs.openSync(evPath, "r+");
  /** page -> {pid, maxRead: Map<file, nLines>} */
  const snaps = new Map();
  let gen = 0;
  /** pages before this one are unchanged in the current generation */
  let firstChanged = 1;
  /** pids of processes running the current generation */
  let running = [];

  const env = {
    ...process.env,
    LLP_NATIVE: path.join(ROOT, "native/llpfork.so"),
    LLP_EVENTS: evPath,
    LLP_RUN: runDir,
    LLP_SHADOW: shadow,
  };

  function tellSnap(pid, cmd) {
    const fd = fs.openSync(path.join(runDir, `snap-${pid}`), "r+");
    fs.writeSync(fd, cmd + "\n");
    fs.closeSync(fd);
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
   * `seedDir` if given. */
  function cold(seedDir) {
    killRunning();
    for (const page of [...snaps.keys()]) dropSnap(page);
    gen += 1;
    firstChanged = 1;
    const dir = path.join(runDir, `g${gen}`);
    fs.mkdirSync(dir);
    for (const f of seedDir ? fs.readdirSync(seedDir) : []) {
      if (!NOT_SEED.has(f)) fs.copyFileSync(path.join(seedDir, f), path.join(dir, f));
    }
    const driver = path.join(ROOT, "engine/driver.lua");
    const tex = `\\directlua{dofile("${driver}")}\\input{${mainTex}}`;
    const p = spawn("bash", ["-c", `exec 200<"$LLP_OUT" && exec lualatex --shell-escape -synctex=-1 -interaction=nonstopmode -jobname=main -output-directory=/proc/self/fd/200 "$0"`, tex], {
      cwd: path.dirname(mainTex),
      env: { ...env, LLP_GEN: String(gen), LLP_OUT: dir },
      stdio: "ignore",
    });
    running = [p.pid];
    log(`gen ${gen}: cold start`);
  }

  /** Recompiles after `file` changed from 1-based line `line` on: resumes the latest
   * snapshot that has not read that line; snapshots that have are dropped after. */
  function edit(file, line) {
    const stale = [...snaps].filter(([, s]) => (s.maxRead.get(file) ?? 0) >= line).map(([page]) => page);
    const best = Math.max(0, ...[...snaps.keys()].filter((page) => !stale.includes(page)));
    if (!best) return cold(null);
    killRunning();
    gen += 1;
    firstChanged = best + 1;
    tellSnap(snaps.get(best).pid, `go ${gen}`);
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
      return;
    }
    switch (kind) {
      case "snap": {
        const [page, pid, mr = ""] = [Number(rest[0]), rest[1], rest.slice(2).join(" ")];
        if (snaps.has(page)) dropSnap(page);
        const maxRead = new Map(mr.split("\t").filter(Boolean).map((kv) => {
          const i = kv.lastIndexOf(":");
          return [kv.slice(0, i), Number(kv.slice(i + 1))];
        }));
        snaps.set(page, { pid, maxRead });
        break;
      }
      case "run": running.push(Number(rest[1])); break;
      case "partial": onPdf({ gen, kind, firstChanged, dir: rest[1] }); break;
      case "done": onPdf({ gen, kind, firstChanged, dir: rest[0] }); running = []; break;
      case "error": log(`gen ${gen} error: ${rest.join(" ")}`); break;
    }
  }

  readline.createInterface({ input: fs.createReadStream(null, { fd: evFd }) }).on("line", onEvent);

  function close() {
    killRunning();
    for (const page of [...snaps.keys()]) dropSnap(page);
  }

  return { cold, edit, writeShadow, close, gen: () => gen };
}

