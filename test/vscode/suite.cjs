// Inside VS Code: open a .tex file, edit it, click the preview, move the cursor; checks each
// effect through the ltxp viewer's HTTP/SSE endpoints.
const assert = require("node:assert/strict");
const http = require("node:http");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const vscode = require("vscode");

const BASE = `http://127.0.0.1:${process.env.LTXP_PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** SSE messages from the viewer endpoint, collected into `events`. */
const events = [];
function listen() {
  http.get(`${BASE}/events`, (res) => res.on("data", (c) => {
    for (const m of c.toString().matchAll(/^data: (.*)$/gm)) events.push(JSON.parse(m[1]));
  })).on("error", () => setTimeout(listen, 200));
}

async function until(what, fn, ms = 60000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const v = await fn();
    if (v) return v;
    await sleep(100);
  }
  throw new Error(`timeout: ${what}`);
}

function get(url) {
  return new Promise((resolve, reject) => http.get(url, (res) => {
    const parts = [];
    res.on("data", (d) => parts.push(d)).on("end", () => resolve(Buffer.concat(parts)));
  }).on("error", reject));
}

function pdfText(buf) {
  return execFileSync("pdftotext", ["-", "-"], { input: buf }).toString();
}

async function run() {
  const dir = process.env.LTXP_TEST_DIR;
  const doc = await vscode.workspace.openTextDocument(path.join(dir, "chap.tex"));
  await vscode.workspace.openTextDocument(path.join(dir, "main.tex"));
  const editor = await vscode.window.showTextDocument(doc);
  assert.equal(doc.languageId, "latex");
  await sleep(2000);
  assert.ok(!(await get(`${BASE}/`).then(() => true, () => false)), "ltxp started without the command");
  await vscode.commands.executeCommand("ltxp.openPreview");
  await until("viewer up", async () => {
    try {
      await get(`${BASE}/`);
      return true;
    } catch {
      return false;
    }
  });
  listen();
  const done = () => events.filter((e) => e.type === "pdf" && e.kind === "done").at(-1);
  await until("first PDF", done);

  // typing: insert a word; the next done PDF must contain it
  const line = doc.getText().split("\n").findIndex((l, i) => i > 10 && /^[a-z]{4,} /.test(l));
  await editor.edit((b) => b.insert(new vscode.Position(line, 0), "zzvscodeedit "));
  await until("edit in PDF", async () => {
    const d = done();
    return d && pdfText(await get(BASE + d.url)).includes("zzvscodeedit");
  });

  // forward search: moving the cursor broadcasts a reveal for this spot
  editor.selection = new vscode.Selection(line, 3, line, 3);
  await until("reveal", () => events.some((e) => e.type === "reveal" && e.after.startsWith("scodeedit")));

  // inverse search: a click on the word puts the cursor on the clicked character
  editor.selection = new vscode.Selection(0, 0, 0, 0);
  const reveal = events.filter((e) => e.type === "reveal").at(-1);
  const body = JSON.stringify({
    url: done().url, page: reveal.page, x: reveal.box.x + 1, y: reveal.box.y + reveal.box.h / 2,
    before: "zzv", after: "scodeedit",
  });
  await new Promise((resolve) => {
    const req = http.request(`${BASE}/click`, { method: "POST" }, (res) => res.resume().on("end", resolve));
    req.end(body);
  });
  await until("cursor jump", () => {
    const s = vscode.window.activeTextEditor?.selection.active;
    return s && s.line === line && s.character === 3;
  }, 10000);
}

module.exports = { run };
