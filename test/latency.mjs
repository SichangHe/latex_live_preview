/** Keystroke-to-PDF latency through the real ltxp server (no editor, no browser): types one
 * character at a time at the start of a line, timing until a PDF from the viewer's event stream
 * contains the typed text. Prints `ms` per keystroke and the ltxp log on a slow one. With
 * `LTXP_VIEWER=1`, times until the browser viewer (headless Chromium) shows the text instead.
 * Usage: node test/latency.mjs DIR MAIN.tex EDIT.tex LINE [N_KEYS] */
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { chromium } from "playwright-core";

const ROOT = path.resolve(import.meta.dirname, "..");
const SLOW_MS = 1000;
const [srcDir, mainName, editName, lineArg, nArg = "8"] = process.argv.slice(2);
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ltxp-lat-"));
const work = path.join(tmp, "work");
fs.cpSync(srcDir, work, { recursive: true });
const editFile = path.join(work, editName);
const uri = pathToFileURL(editFile).href;
let text = fs.readFileSync(editFile, "utf8");
const errLog = path.join(tmp, "ltxp.err");
const server = spawn(path.join(ROOT, "target/release/ltxp"), [], { stdio: ["pipe", "pipe", fs.openSync(errLog, "w")] });
process.on("exit", () => server.kill("SIGTERM"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const send = (msg) => {
  const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", ...msg }));
  server.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
  server.stdin.write(body);
};
let buf = Buffer.alloc(0);
let url = null;
server.stdout.on("data", (c) => {
  buf = Buffer.concat([buf, c]);
  for (;;) {
    const sep = buf.indexOf("\r\n\r\n");
    if (sep < 0) return;
    const len = Number(/Content-Length: *(\d+)/i.exec(buf.subarray(0, sep).toString())[1]);
    if (buf.length < sep + 4 + len) return;
    const msg = JSON.parse(buf.subarray(sep + 4, sep + 4 + len).toString());
    buf = buf.subarray(sep + 4 + len);
    if (msg.id === 2 && typeof msg.result === "string") url = msg.result;
    if (msg.id !== undefined && msg.method) send({ id: msg.id, result: { success: true } });
  }
});
send({ id: 1, method: "initialize", params: { capabilities: {}, initializationOptions: { openBrowser: false } } });
const open = (f) => send({ method: "textDocument/didOpen", params: { textDocument: { uri: pathToFileURL(f).href, languageId: "latex", version: 1, text: fs.readFileSync(f, "utf8") } } });
if (editName !== mainName) open(path.join(work, mainName));
open(editFile);
while (!url) {
  await sleep(100);
  send({ id: 2, method: "workspace/executeCommand", params: { command: "ltxp.previewUrl", arguments: [] } });
}
const pdfs = [];
let lastEventMs = Date.now();
http.get(new URL("/events", url), (res) => res.on("data", (c) => {
  for (const m of c.toString().matchAll(/^data: (.*)$/gm)) {
    const msg = JSON.parse(m[1]);
    if (msg.type !== "pdf") continue;
    lastEventMs = Date.now();
    pdfs.push({ ...msg, at: Date.now() });
  }
}));
const get = (u) => new Promise((resolve) => http.get(new URL(u, url), (res) => {
  const parts = [];
  res.on("data", (d) => parts.push(d)).on("end", () => resolve(Buffer.concat(parts)));
}));
while (!pdfs.some((p) => p.kind === "done") || Date.now() - lastEventMs < 4000) await sleep(100);

const line = Number(lineArg) - 1;
let page = null;
if (process.env.LTXP_VIEWER) {
  const exe = process.env.LTXP_CHROME ?? path.join(os.homedir(), ".cache/ms-playwright/chromium-1200/chrome-linux64/chrome");
  const browser = await chromium.launch({ executablePath: exe });
  process.on("exit", () => browser.close());
  page = await browser.newPage({ viewport: { width: 1000, height: 1200 } });
  if (process.env.LTXP_CONSOLE) page.on("console", (m) => m.text().startsWith("T ") && console.log(m.text()));
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector(".textLayer"), null, { timeout: 60000 });
  send({ method: "ltxp/cursor", params: { textDocument: { uri }, position: { line, character: 0 } } });
  await page.waitForFunction(() => document.querySelector(".marker"), null, { timeout: 20000 });
  await sleep(1000);
}
let version = 1;
let typed = "";
function insert(ch) {
  send({ method: "textDocument/didChange", params: { textDocument: { uri, version: ++version }, contentChanges: [{ range: { start: { line, character: typed.length }, end: { line, character: typed.length } }, text: ch }] } });
  typed += ch;
}
insert("z");
insert("q");
await sleep(3000);
const word = "zqedittimexy".slice(0, 2 + Number(nArg));
for (const ch of word.slice(2)) {
  const t0 = Date.now();
  const seen = pdfs.length;
  insert(ch);
  let ms = null;
  if (page) {
    await page.waitForFunction((w) => [...document.querySelectorAll(".textLayer")].some((t) => t.textContent.includes(w)), typed, { timeout: 30000, polling: 5 }).then(() => (ms = Date.now() - t0), () => {});
  }
  for (let i = seen; !page && ms === null && Date.now() - t0 < 30000; ) {
    if (i >= pdfs.length) {
      await sleep(5);
      continue;
    }
    const p = pdfs[i++];
    const pdf = await get(p.url);
    if (pdf.length && execFileSync("pdftotext", ["-", "-"], { input: pdf }).toString().includes(typed)) ms = p.at - t0;
  }
  const lines = fs.readFileSync(errLog, "utf8").split("\n");
  console.log(`${ch} ${ms ?? "timeout"} ms (sent at ${t0 % 100000})`);
  if (ms === null || ms > SLOW_MS || process.env.LTXP_DEBUG) console.log(lines.slice(-Number(process.env.LTXP_TAIL ?? 8)).join("\n"));
  await sleep(1500);
}
fs.rmSync(tmp, { recursive: true, force: true });
process.exit(0);
