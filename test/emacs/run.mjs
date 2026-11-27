/** Real Emacs 30 (docker `silex/emacs:30`) with ltxp.el and the release ltxp binary: typing
 * reaches the PDF, point moves mark the preview, and preview clicks move point exactly.
 * The server runs on the host via `chroot /host`. Usage: node test/emacs/run.mjs */
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "../..");
const PORT = 47312;
const BASE = `http://127.0.0.1:${PORT}`;
const work = fs.mkdtempSync(path.join(os.tmpdir(), "ltxp-emacs-"));
fs.cpSync(path.join(ROOT, "test/multifile"), work, { recursive: true });
fs.chmodSync(work, 0o777);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const name = `ltxp-emacs-${process.pid}`;
const uid = `${process.getuid()}:${process.getgid()}`;
const plugin = path.join(ROOT, "editors/emacs");
const emacs = spawn("docker", [
  "run", "--rm", "--name", name, "--network", "host", "-v", "/:/host", "-v", `${work}:${work}`, "-v", `${plugin}:${plugin}:ro`, "-v", `${import.meta.dirname}:/drive:ro`,
  "-e", `LTXP_PORT=${PORT}`, "-e", `LTXP_TEST_DIR=${work}`, "-e", `LTXP_PLUGIN=${plugin}/ltxp.el`,
  "-e", `LTXP_TEST_CMD=chroot --userspec=${uid} /host env PATH=${process.env.PATH} HOME=${os.homedir()} ${ROOT}/target/release/ltxp`,
  "silex/emacs:30", "emacs", "--batch", "-l", "/drive/drive.el",
], { stdio: ["ignore", "ignore", fs.openSync(path.join(work, "emacs.err"), "w")] });
const fail = (m) => {
  console.log(`FAIL emacs: ${m}; kept ${work}`);
  execFileSync("docker", ["rm", "-f", name], { stdio: "ignore" });
  process.exit(1);
};
const cmd = (c) => fs.appendFileSync(path.join(work, "cmd"), c + "\n");
const read = (f) => { try { return fs.readFileSync(path.join(work, f), "utf8"); } catch { return ""; } };
async function until(what, fn, ms = 60000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const v = await fn();
    if (v) return v;
    await sleep(100);
  }
  fail(`timeout: ${what}`);
}
const get = (url) => new Promise((resolve, reject) => http.get(url, (res) => {
  const parts = [];
  res.on("data", (d) => parts.push(d)).on("end", () => resolve(Buffer.concat(parts)));
}).on("error", reject));
const events = [];
const listen = () => http.get(`${BASE}/events`, (res) => res.on("data", (c) => {
  for (const m of c.toString().matchAll(/^data: (.*)$/gm)) events.push(JSON.parse(m[1]));
})).on("error", () => setTimeout(listen, 200));
const done = () => events.filter((e) => e.type === "pdf" && e.kind === "done").at(-1);

await until("eglot connected", () => read("status") === "ready");
await until("viewer up", () => get(`${BASE}/`).then(() => true, () => false));
listen();
await until("first PDF", done);
const line = read("chap.tex").split("\n").findIndex((l, i) => i > 10 && /^[a-z]{4,} /.test(l));
cmd(`insert ${line} zzemacsedit`);
await until("edit in PDF", async () => {
  const d = done();
  return d && execFileSync("pdftotext", ["-", "-"], { input: await get(BASE + d.url) }).toString().includes("zzemacsedit");
});
cmd(`move ${line} 3`);
const reveal = await until("reveal", () => events.find((e) => e.type === "reveal" && e.after.startsWith("macsedit")));
cmd("move 0 0");
await until("point home", () => read("point") === "0 0");
await new Promise((resolve) => http.request(`${BASE}/click`, { method: "POST" }, (res) => res.resume().on("end", resolve))
  .end(JSON.stringify({ url: done().url, page: reveal.page, x: reveal.box.x + 1, y: reveal.box.y + reveal.box.h / 2, before: "zze", after: "macsedit" })));
await until("point jump", () => read("point") === `${line} 3`, 10000);
cmd("quit");
await new Promise((r) => emacs.on("exit", r));
await sleep(1000);
if (fs.readdirSync("/dev/shm").some((d) => d.startsWith("ltxp-"))) fail("run dir left behind");
fs.rmSync(work, { recursive: true, force: true });
