#!/usr/bin/env node
// 🧑 "Layout using. Latex engine, but display using a web browser. Doing some sort of. Language server or editor plugin. Ideally, mainly language server."
/** `llp`: LaTeX live-preview language server with a browser viewer; see docs/index.md. */
import { execFile } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createEngine, seedSig } from "./engine.js";
import { lspTransport } from "./lsp.js";
import { forward, inverse } from "./sync.js";

const ROOT = path.resolve(import.meta.dirname, "..");
const PORT = Number(process.env.LLP_PORT ?? 0);
const MAX_AUTO_RERUNS = 3;
const GC_INTERVAL_MS = 10_000;
const STATIC = {
  "/": [path.join(ROOT, "viewer/index.html"), "text/html"],
  "/viewer.js": [path.join(ROOT, "viewer/viewer.js"), "text/javascript"],
  "/match.js": [path.join(ROOT, "server/match.js"), "text/javascript"],
  "/pdf.mjs": [path.join(ROOT, "node_modules/pdfjs-dist/build/pdf.min.mjs"), "text/javascript"],
  "/pdf.worker.mjs": [path.join(ROOT, "node_modules/pdfjs-dist/build/pdf.worker.min.mjs"), "text/javascript"],
  "/pdf_viewer.css": [path.join(ROOT, "node_modules/pdfjs-dist/web/pdf_viewer.css"), "text/css"],
};

const log = (m) => process.stderr.write(`[llp] ${m}\n`);
/** uri -> {file, text} for documents open in the editor */
const docs = new Map();
let project = null;
let nextId = 1;

const send = lspTransport(onLsp);

function readText(file) {
  for (const d of docs.values()) if (d.file === file) return d.text;
  try { return fs.readFileSync(file, "utf8"); } catch { return null; }
}

/** `% !TEX root = x` magic comment, else the file itself if it has `\documentclass`. */
function findMain(file, text) {
  const m = /^%\s*!TEX root\s*=\s*(.+?)\s*$/im.exec(text);
  if (m) return path.resolve(path.dirname(file), m[1]);
  return /^[^%\n]*\\documentclass/m.test(text) ? file : null;
}

function startProject(mainTex) {
  const runDir = fs.mkdtempSync(path.join(fs.existsSync("/dev/shm") ? "/dev/shm" : os.tmpdir(), "llp-"));
  const clients = new Set();
  const p = { mainTex, runDir, clients, latest: {}, autoReruns: 0, diagnosed: new Set() };
  p.engine = createEngine({ mainTex, runDir, log, onPdf: (pdf) => onPdf(p, pdf) });
  p.server = http.createServer((req, res) => onHttp(p, req, res)).listen(PORT, "127.0.0.1", () => {
    const url = `http://127.0.0.1:${p.server.address().port}/`;
    log(`viewer at ${url}`);
    send({ method: "window/showMessage", params: { type: 3, message: `LaTeX preview: ${url}` } });
    send({ id: nextId++, method: "window/showDocument", params: { uri: url, external: true } });
  });
  setInterval(() => gcRuns(p), GC_INTERVAL_MS).unref();
  for (const d of docs.values()) p.engine.writeShadow(d.file, d.text);
  p.engine.cold(null);
  return p;
}

function broadcast(p, msg) {
  for (const res of p.clients) res.write(`data: ${JSON.stringify(msg)}\n\n`);
}

function pdfMsg({ gen, kind, firstChanged, dir }) {
  return { type: "pdf", gen, kind, firstChanged, url: `/out/${path.basename(dir)}/main.pdf` };
}

function onPdf(p, pdf) {
  const { kind, dir } = pdf;
  p.latest[kind] = pdf;
  broadcast(p, pdfMsg(pdf));
  if (kind === "done") {
    publishDiagnostics(p, dir);
    rerunIfStale(p, pdf, false);
  }
}

/** Sends the errors of the run in `dir` (`-file-line-error` log lines) for each project file. */
function publishDiagnostics(p, dir) {
  const log = readOr(path.join(dir, "main.log"), "");
  const root = path.dirname(p.mainTex) + path.sep;
  const byFile = new Map([...p.diagnosed].map((f) => [f, []]));
  for (const m of log.matchAll(/^(.+?\.\w+):(\d+): (.*)$/gm)) {
    const file = path.resolve(path.dirname(p.mainTex), m[1]);
    if (!file.startsWith(root)) continue;
    const line = Number(m[2]) - 1;
    const range = { start: { line, character: 0 }, end: { line, character: 1000 } };
    byFile.set(file, [...(byFile.get(file) ?? []), { range, severity: 1, source: "llp", message: m[3] }]);
  }
  p.diagnosed = new Set([...byFile].filter(([, d]) => d.length).map(([f]) => f));
  for (const [file, diagnostics] of byFile) {
    send({ method: "textDocument/publishDiagnostics", params: { uri: pathToFileURL(file).href, diagnostics } });
  }
}

/** Cross-references and citations settle over runs: when a finished run wrote auxiliary
 * files different from those it started with, run bibtex/biber if needed and compile again. */
function rerunIfStale(p, { dir, seed }, force) {
  if (!force && (seedSig(dir) === seed || p.autoReruns >= MAX_AUTO_RERUNS)) return;
  p.autoReruns += 1;
  const aux = readOr(path.join(dir, "main.aux"), "");
  const bib = fs.existsSync(path.join(dir, "main.bcf")) ? "biber" : aux.includes("\\bibdata") ? "bibtex" : null;
  const env = { ...process.env, BIBINPUTS: `${path.dirname(p.mainTex)}:`, BSTINPUTS: `${path.dirname(p.mainTex)}:` };
  const gen = p.engine.gen();
  const recompile = () => p.engine.gen() === gen && p.engine.cold(dir);
  if (bib) execFile(bib, ["main"], { cwd: dir, env }, recompile);
  else recompile();
}

/** Removes run dirs the viewer no longer needs. */
function gcRuns(p) {
  const keep = new Set([p.latest.done?.dir, p.latest.partial?.dir].filter(Boolean).map((d) => path.basename(d)));
  const live = `g${p.engine.gen()}`;
  for (const name of fs.readdirSync(p.runDir)) {
    if (!/^g\d+(p\d+)?$/.test(name) || keep.has(name) || name === live || name.startsWith(live + "p")) continue;
    const full = path.join(p.runDir, name);
    if (Date.now() - fs.statSync(full).mtimeMs > GC_INTERVAL_MS) fs.rmSync(full, { recursive: true, force: true });
  }
}

function onHttp(p, req, res) {
  if (req.headers.host !== `127.0.0.1:${p.server.address().port}`) return res.writeHead(403).end();
  const url = new URL(req.url, "http://x");
  if (url.pathname === "/events") {
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
    p.clients.add(res);
    for (const l of [p.latest.done, p.latest.partial]) if (l) res.write(`data: ${JSON.stringify({ ...pdfMsg(l), firstChanged: 1 })}\n\n`);
    req.on("close", () => p.clients.delete(res));
    return;
  }
  if (url.pathname === "/click" && req.method === "POST") {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => onClick(p, JSON.parse(body)).catch((e) => log(`click: ${e}`)).finally(() => res.end()));
    return;
  }
  const out = /^\/out\/(g\d+(?:p\d+)?)\/main\.pdf$/.exec(url.pathname);
  const [file, type] = out ? [path.join(p.runDir, out[1], "main.pdf"), "application/pdf"] : STATIC[url.pathname] ?? [];
  if (!file) return res.writeHead(404).end();
  fs.stat(file, (err, st) => {
    if (err) return res.writeHead(404).end();
    const head = { "content-type": type, "cache-control": "no-store", "accept-ranges": "bytes" };
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? "");
    if (!range) return fs.createReadStream(file).on("error", () => res.destroy()).pipe(res.writeHead(200, { ...head, "content-length": st.size }));
    const [start, end] = [Number(range[1]), Math.min(st.size - 1, range[2] ? Number(range[2]) : st.size - 1)];
    if (start > end) return res.writeHead(416).end();
    res.writeHead(206, { ...head, "content-length": end - start + 1, "content-range": `bytes ${start}-${end}/${st.size}` });
    fs.createReadStream(file, { start, end }).on("error", () => res.destroy()).pipe(res);
  });
}

/** Viewer click: jump the editor to the exact source character. */
async function onClick(p, { url, page, x, y, before, after }) {
  const dir = path.join(p.runDir, path.basename(path.dirname(url)));
  if (!fs.existsSync(path.join(dir, "main.synctex"))) return;
  const r = await inverse({ pdf: path.join(dir, "main.pdf"), page, x, y, before, after }, readText);
  if (!r) return;
  const pos = { line: r.line, character: r.col };
  send({
    id: nextId++,
    method: "window/showDocument",
    params: { uri: pathToFileURL(r.file).href, takeFocus: true, selection: { start: pos, end: pos } },
  });
}

/** Editor cursor: show the exact spot in the viewer. */
async function reveal(uri, position) {
  const p = project;
  const doc = docs.get(uri);
  const { done, partial } = p?.latest ?? {};
  const l = partial && (!done || partial.gen > done.gen) ? partial : done;
  if (!doc || !l) return;
  const r = await forward({ pdf: path.join(l.dir, "main.pdf"), file: doc.file, line: position.line, col: position.character }, readText);
  if (r) broadcast(p, { type: "reveal", ...r });
}

/** Applies LSP incremental changes; returns the first changed 0-based line. */
function applyChanges(doc, changes) {
  let first = Infinity;
  for (const c of changes) {
    if (!c.range) {
      first = 0;
      doc.text = c.text;
      continue;
    }
    const starts = [0];
    for (let i = 0; i < doc.text.length; i++) if (doc.text[i] === "\n") starts.push(i + 1);
    const off = (pos) => Math.min(doc.text.length, (starts[pos.line] ?? doc.text.length) + pos.character);
    doc.text = doc.text.slice(0, off(c.range.start)) + c.text + doc.text.slice(off(c.range.end));
    first = Math.min(first, c.range.start.line);
  }
  return first;
}

function onEdit(doc, firstLine) {
  if (!project) {
    const main = findMain(doc.file, doc.text);
    if (main) project = startProject(main);
    return;
  }
  if (doc.file.endsWith(".bib")) return;
  project.autoReruns = 0;
  project.engine.writeShadow(doc.file, doc.text);
  project.engine.edit(doc.file, firstLine + 1);
}

function onLsp(msg) {
  const { id, method, params } = msg;
  switch (method) {
    case "initialize":
      return send({
        id,
        result: {
          capabilities: {
            textDocumentSync: { openClose: true, change: 2, save: true },
            executeCommandProvider: { commands: ["llp.forwardSearch"] },
          },
          serverInfo: { name: "llp" },
        },
      });
    case "textDocument/didOpen": {
      const { uri, text } = params.textDocument;
      const doc = { file: path.normalize(fileURLToPath(uri)), text };
      docs.set(uri, doc);
      const disk = readOr(doc.file, "");
      if (!project || disk !== text) onEdit(doc, firstDiffLine(disk, text));
      return;
    }
    case "textDocument/didChange": {
      const doc = docs.get(params.textDocument.uri);
      if (doc) onEdit(doc, applyChanges(doc, params.contentChanges));
      return;
    }
    case "textDocument/didSave": {
      const file = fileURLToPath(params.textDocument.uri);
      if (project?.latest.done && file.endsWith(".bib")) {
        project.autoReruns = 0;
        rerunIfStale(project, project.latest.done, true);
      }
      return;
    }
    case "textDocument/didClose": {
      const doc = docs.get(params.textDocument.uri);
      docs.delete(params.textDocument.uri);
      if (!project || !doc) return;
      const disk = readOr(doc.file, "");
      project.engine.dropShadow(doc.file);
      if (disk !== doc.text) project.engine.edit(doc.file, firstDiffLine(doc.text, disk) + 1);
      return;
    }
    case "llp/cursor":
      return void reveal(params.textDocument.uri, params.position);
    case "workspace/executeCommand":
      if (params.command === "llp.forwardSearch") reveal(params.arguments[0], params.arguments[1]);
      return send({ id, result: null });
    case "shutdown":
      return send({ id, result: null });
    case "exit":
      return quit();
    default:
      if (id !== undefined && method) send({ id, error: { code: -32601, message: `unsupported: ${method}` } });
  }
}

function readOr(file, fallback) {
  try { return fs.readFileSync(file, "utf8"); } catch { return fallback; }
}

/** Kills all TeX processes and removes scratch files; also on editor crash or signals. */
function quit() {
  project?.engine.close();
  if (project) fs.rmSync(project.runDir, { recursive: true, force: true });
  process.exit(0);
}
process.stdin.on("end", quit);
for (const sig of ["SIGTERM", "SIGHUP", "SIGINT"]) process.on(sig, quit);
process.on("uncaughtException", (e) => log(`error: ${e.stack}`));

function firstDiffLine(a, b) {
  const la = a.split("\n");
  const lb = b.split("\n");
  let i = 0;
  while (i < la.length && i < lb.length && la[i] === lb[i]) i++;
  return i;
}
