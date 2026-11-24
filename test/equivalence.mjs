/** Checks that the live preview equals a from-scratch `latexmk -lualatex` build, initially and
 * after incremental edits to EDIT.tex (default MAIN.tex).
 * Usage: node test/equivalence.mjs DIR MAIN.tex [EDIT.tex]
 * Prints only failures and one summary line. */
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const ROOT = path.resolve(import.meta.dirname, "..");
const SETTLE_MS = 4000;
const [srcDir, mainName, editName = mainName] = process.argv.slice(2);
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "llp-eq-"));
const work = path.join(tmp, "work");
fs.cpSync(srcDir, work, { recursive: true });
const mainFile = path.join(work, mainName);
const editFile = path.join(work, editName);
const uri = pathToFileURL(editFile).href;
let text = fs.readFileSync(editFile, "utf8");
let failed = false;
const fail = (m) => {
  failed = true;
  console.log(`FAIL ${path.basename(srcDir)}: ${m}`);
};

const server = spawn(path.join(ROOT, "server/main.js"), [], { stdio: ["pipe", "pipe", fs.openSync(path.join(tmp, "llp.err"), "w")] });
const send = (msg) => {
  const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", ...msg }));
  server.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
  server.stdin.write(body);
};
let viewerUrl = null;
let buf = Buffer.alloc(0);
server.stdout.on("data", (c) => {
  buf = Buffer.concat([buf, c]);
  for (;;) {
    const sep = buf.indexOf("\r\n\r\n");
    if (sep < 0) return;
    const len = Number(/Content-Length: *(\d+)/i.exec(buf.subarray(0, sep).toString())[1]);
    if (buf.length < sep + 4 + len) return;
    const msg = JSON.parse(buf.subarray(sep + 4, sep + 4 + len).toString());
    buf = buf.subarray(sep + 4 + len);
    if (msg.method === "window/showDocument" && msg.params.external) viewerUrl = msg.params.uri;
    if (msg.id !== undefined && msg.method) send({ id: msg.id, result: { success: true } });
  }
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

send({ id: 1, method: "initialize", params: { capabilities: {} } });
send({ method: "initialized", params: {} });
const open = (u, t) => send({ method: "textDocument/didOpen", params: { textDocument: { uri: u, languageId: "latex", version: 1, text: t } } });
if (editFile !== mainFile) open(pathToFileURL(mainFile).href, fs.readFileSync(mainFile, "utf8"));
open(uri, text);
while (!viewerUrl) await sleep(50);

let latestDone = null;
let lastEventMs = Date.now();
http.get(new URL("/events", viewerUrl), (res) => {
  res.on("data", (c) => {
    for (const m of c.toString().matchAll(/^data: (.*)$/gm)) {
      const msg = JSON.parse(m[1]);
      if (msg.type !== "pdf") continue;
      lastEventMs = Date.now();
      if (msg.kind === "done") latestDone = msg;
    }
  });
});

/** Waits for a complete PDF newer than the one used last, then until no PDF event came for
 * `SETTLE_MS`; returns that PDF's text. */
let usedGen = 0;
async function settledText() {
  const t0 = Date.now();
  while (!latestDone || latestDone.gen <= usedGen || Date.now() - lastEventMs < SETTLE_MS) {
    if (Date.now() - t0 > 300_000) throw new Error("no settled preview in 300 s");
    await sleep(200);
  }
  const pdf = path.join(tmp, "preview.pdf");
  const data = await new Promise((resolve) => http.get(new URL(latestDone.url, viewerUrl), (res) => {
    const parts = [];
    res.on("data", (d) => parts.push(d)).on("end", () => resolve(Buffer.concat(parts)));
  }));
  fs.writeFileSync(pdf, data);
  usedGen = latestDone.gen;
  return pdfText(pdf);
}

/** Per page: words with positions, and a hash of a low-resolution grayscale rendering. */
function pdfText(pdf) {
  const html = execFileSync("pdftotext", ["-bbox", pdf, "-"], { maxBuffer: 1 << 28 }).toString();
  const text = html.split("<page ").slice(1).map((pg) => [...pg.matchAll(/<word xMin="([\d.]+)" yMin="([\d.]+)"[^>]*>([^<]*)</g)]
    .map((m) => `${Number(m[1]).toFixed(2)},${Number(m[2]).toFixed(2)} ${m[3]}`).join("\n"));
  const dir = fs.mkdtempSync(path.join(tmp, "px-"));
  execFileSync("pdftoppm", ["-r", "30", "-gray", pdf, path.join(dir, "p")]);
  const px = fs.readdirSync(dir).sort().map((f) => createHash("sha1").update(fs.readFileSync(path.join(dir, f))).digest("hex"));
  fs.rmSync(dir, { recursive: true });
  return text.map((t, i) => ({ t, px: px[i] }));
}

function referenceText() {
  const ref = path.join(tmp, "ref");
  fs.rmSync(ref, { recursive: true, force: true });
  fs.cpSync(srcDir, ref, { recursive: true });
  fs.writeFileSync(path.join(ref, editName), text);
  try {
    execFileSync("latexmk", ["-lualatex", "-f", "-interaction=nonstopmode", mainName], { cwd: ref, stdio: "ignore", timeout: 300_000 });
  } catch {}
  return pdfText(path.join(ref, mainName.replace(/\.tex$/, ".pdf")));
}

function compare(label) {
  return settledText().then((got) => {
    const want = referenceText();
    if (got.length !== want.length) return fail(`${label}: ${got.length} pages, reference ${want.length}`);
    const i = got.findIndex((p, k) => p.t !== want[k].t);
    if (i < 0) {
      const k = got.findIndex((p, k) => p.px !== want[k].px);
      if (k >= 0) fail(`${label}: page ${k + 1} text equal but rendering differs`);
      return;
    }
    const gl = got[i].t.split("\n");
    const wl = want[i].t.split("\n");
    const j = gl.findIndex((l, k) => l !== wl[k]);
    fail(`${label}: page ${i + 1} word ${j + 1} differs:\n  got  ${JSON.stringify(gl[j])}\n  want ${JSON.stringify(wl[j])}`);
  });
}

let version = 1;
function edit(line, newLineText) {
  const lines = text.split("\n");
  const old = lines[line];
  lines[line] = newLineText;
  text = lines.join("\n");
  send({
    method: "textDocument/didChange",
    params: {
      textDocument: { uri, version: ++version },
      contentChanges: [{ range: { start: { line, character: 0 }, end: { line, character: old.length } }, text: newLineText }],
    },
  });
}

await compare("initial");
const lines = text.split("\n");
const bodyStart = lines.findIndex((l) => l.includes("\\begin{document}"));
const prose = lines.map((l, i) => i).filter((i) => i > bodyStart && /^[A-Za-z][^%]{40,}$/.test(lines[i]));
if (prose.length < 2) fail("no prose lines to edit");
else {
  const a = prose[Math.floor(prose.length / 2)];
  edit(a, "Inserted words for the incremental test. " + lines[a]);
  await compare("after insert");
  const b = prose[Math.floor(prose.length * 0.8)];
  edit(b, "");
  await compare("after delete");
  const c = prose[Math.floor(prose.length * 0.3)];
  const orig = text.split("\n")[c];
  for (const broken of [" \\textbf{unclosed", " $x^", " \\begin{itemize}", " \\undefinedmacro", " }"]) {
    edit(c, orig + broken);
    await sleep(700);
  }
  edit(c, orig + " fixed");
  await compare("after errors then fix");
}
send({ id: 2, method: "shutdown" });
send({ method: "exit" });
await sleep(500);
if (!failed) console.log(`ok ${path.basename(srcDir)}`);
process.exit(failed ? 1 : 0);
