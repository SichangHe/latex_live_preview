/** End-to-end test with real Neovim, real `ltxp`, and real Chromium; prints only failures and
 * a summary. Usage: node test/e2e.mjs [TEX_FILE] (default: generated 200-section doc). */
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { chromium } from "playwright-core";

const ROOT = path.resolve(import.meta.dirname, "..");
const CHROME = process.env.LTXP_CHROME ?? path.join(os.homedir(), ".cache/ms-playwright/chromium-1200/chrome-linux64/chrome");
const N_CLICKS = 40;
const N_FORWARD = 20;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ltxp-e2e-"));
const sock = path.join(tmp, "nvim.sock");
let texFile = process.argv[2];
if (!texFile) {
  texFile = path.join(tmp, "main.tex");
  execFileSync("python3", [path.join(ROOT, "test/gen_doc.py"), texFile, "200"]);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const failures = [];
const fail = (m) => {
  failures.push(m);
  console.log("FAIL", m);
};

fs.writeFileSync(path.join(tmp, "init.lua"), `
vim.opt.rtp:prepend(${JSON.stringify(path.join(ROOT, "editors/nvim"))})
vim.ui.open = function(url) vim.g.ltxp_url = url; return nil, nil end
require("ltxp").setup({ cmd = { ${JSON.stringify(path.join(ROOT, "target/release/ltxp"))} } })
`);
const nvimProc = spawn("nvim", ["--headless", "--listen", sock, "-u", path.join(tmp, "init.lua"), texFile], {
  stdio: ["ignore", "ignore", fs.openSync(path.join(tmp, "nvim.err"), "w")],
});
const lua = (code) => JSON.parse(execFileSync("nvim", ["--server", sock, "--remote-expr", `json_encode(luaeval(${JSON.stringify(code)}))`]).toString());
const keys = (k) => execFileSync("nvim", ["--server", sock, "--remote-send", k]);

async function waitFor(fn, ms, what) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {}
    await sleep(20);
  }
  throw new Error(`timeout: ${what}`);
}

const url = await waitFor(() => fs.existsSync(sock) && lua("vim.g.ltxp_url"), 20000, "viewer url");
const browser = await chromium.launch({ executablePath: CHROME });
const page = await browser.newPage({ viewport: { width: 1000, height: 1200 } });
await page.goto(url);
await page.waitForFunction(() => document.getElementById("status").textContent.includes("done"), null, { timeout: 60000 });
const lines = fs.readFileSync(texFile, "utf8").split("\n");

/** Source position of q-word `w` (0-based line, col), or null if not unique. */
function wordPos(w) {
  const re = new RegExp(`(?<![a-z])${w}(?![a-z])`);
  const hits = [];
  lines.forEach((l, i) => {
    const m = re.exec(l);
    if (m) hits.push({ line: i, col: m.index });
  });
  return hits.length === 1 ? hits[0] : null;
}

// typing latency: after reruns settle, type on a mid-document line; time each keystroke
// until the viewer shows it
const status = () => page.evaluate(() => document.getElementById("status").textContent);
for (let last = "", same = 0; same < 15; await sleep(200)) {
  const s = await status();
  same = s === last ? same + 1 : 0;
  last = s;
}
const editLine = lines.findIndex((l, i) => i > lines.length * 0.6 && /^q[a-z]+ /.test(l));
lua(`vim.api.nvim_win_set_cursor(0, {${editLine + 1}, 0})`);
await page.waitForFunction(() => document.querySelector(".marker"), null, { timeout: 10000 }).catch(() => fail("forward reveal before edit"));
keys("Izz");
const latenciesMs = [];
let typed = "zz";
for (const ch of "editxyz") {
  typed += ch;
  const t = Date.now();
  keys(ch);
  await page.waitForFunction((w) => [...document.querySelectorAll(".textLayer")].some((t) => t.textContent.includes(w)), typed, { timeout: 20000, polling: 5 });
  latenciesMs.push(Date.now() - t);
}
keys(" <Esc>");
lines[editLine] = typed + " " + lines[editLine];
const latencyMs = latenciesMs.join(",");

// inverse search: click on random characters of random words on rendered pages
let exact = 0, sameLine = 0, tried = 0;
for (let c = 0; c < N_CLICKS; c++) {
  const pick = await page.evaluate((seed) => {
    const vis = [...document.querySelectorAll(".page")].filter((p) => {
      const r = p.getBoundingClientRect();
      return r.bottom > 0 && r.top < innerHeight && p.querySelector(".textLayer span");
    });
    const p = vis[seed % vis.length];
    const spans = [...p.querySelectorAll(".textLayer span")].filter((s) => /q[a-z]{3,}/.test(s.textContent));
    const s = spans[(seed * 7919) % spans.length];
    const words = [...s.textContent.matchAll(/q[a-z]{3,}/g)];
    const m = words[(seed * 31) % words.length];
    const k = 1 + (seed % (m[0].length - 1));
    const r = document.createRange();
    r.setStart(s.firstChild, m.index + k);
    r.setEnd(s.firstChild, m.index + k + 1);
    const rect = r.getBoundingClientRect();
    if (rect.top < 0 || rect.bottom > innerHeight) return null;
    return { word: m[0], k, x: rect.left + 1, y: rect.top + rect.height / 2 };
  }, c * 104729 + 17);
  if (!pick) continue;
  const want = wordPos(pick.word);
  if (!want) continue;
  tried++;
  lua(`vim.api.nvim_win_set_cursor(0, {1, 0})`);
  await page.mouse.click(pick.x, pick.y);
  const got = await waitFor(() => {
    const [l, col] = lua("vim.api.nvim_win_get_cursor(0)");
    return l !== 1 || col !== 0 ? { line: l - 1, col } : null;
  }, 3000, "cursor jump").catch(() => null);
  if (got && got.line === want.line && got.col === want.col + pick.k) exact++;
  else if (got && got.line === want.line) {
    sameLine++;
    fail(`click ${pick.word}[${pick.k}] -> col ${got.col}, want ${want.col + pick.k} (line ${want.line + 1})`);
  } else fail(`click ${pick.word}[${pick.k}] -> ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  if (c % 10 === 9) await page.mouse.wheel(0, 3000), await sleep(400);
}

// forward search: put the editor cursor on a word character, check the caret marker lands on it
let fwdExact = 0, fwdTried = 0;
for (let c = 0; c < N_FORWARD; c++) {
  const li = Math.floor(((c + 1) / (N_FORWARD + 1)) * lines.length * 0.8) + 3;
  const m = /(?<![a-z\\])q[a-z]{4,}/.exec(lines[li].slice(5));
  if (!m) continue;
  const col = 5 + m.index + 2;
  fwdTried++;
  await page.evaluate(() => document.querySelectorAll(".marker").forEach((x) => x.remove()));
  lua(`vim.api.nvim_win_set_cursor(0, {${li + 1}, ${col}})`);
  const ok = await page.waitForFunction((w) => {
    const mk = document.querySelector(".marker.caret");
    if (!mk) return null;
    const mr = mk.getBoundingClientRect();
    for (const s of mk.parentElement.querySelectorAll(".textLayer span")) {
      const i = s.textContent.indexOf(w);
      if (i < 0) continue;
      const r = document.createRange();
      r.setStart(s.firstChild, i + 2);
      r.setEnd(s.firstChild, i + 3);
      const cr = r.getBoundingClientRect();
      return Math.abs(cr.left - mr.left) < 2 && Math.abs(cr.top - mr.top) < 4 ? "ok" : `off by ${(mr.left - cr.left).toFixed(1)},${(mr.top - cr.top).toFixed(1)}`;
    }
    return "word not on marker page";
  }, m[0], { timeout: 5000 }).then((h) => h.jsonValue()).catch(() => "no marker");
  if (ok === "ok") fwdExact++;
  else fail(`forward ${m[0]}[2] line ${li + 1}: ${ok}`);
}

console.log(`typing latency ${latencyMs} ms; inverse exact ${exact}/${tried} (same line only ${sameLine}); forward exact ${fwdExact}/${fwdTried}`);
await browser.close();
keys("<Esc>:qa!<CR>");
await sleep(500);
nvimProc.kill();
process.exit(failures.length ? 1 : 0);
