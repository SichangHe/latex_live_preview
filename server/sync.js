/** SyncTeX queries refined to exact characters; see docs/sync.md. */
import { execFile } from "node:child_process";
import path from "node:path";
import { bestSplit, letters } from "./match.js";

const WINDOW_LINES = 6;
const CONTEXT_CHARS = 40;
/** Below this fraction of context letters matched near the SyncTeX line, search the file. */
const MIN_FIT = 0.8;

function synctex(args) {
  return new Promise((resolve) => {
    execFile("synctex", args, (err, out) => resolve(err ? [] : parseRecords(out)));
  });
}

/** Splits `synctex` CLI output into records, each starting at an `Output:` line. */
function parseRecords(out) {
  const recs = [];
  for (const l of out.split("\n")) {
    const i = l.indexOf(":");
    if (i < 0) continue;
    const [k, v] = [l.slice(0, i), l.slice(i + 1)];
    if (k === "Output") recs.push({});
    else if (recs.length) recs.at(-1)[k] = v;
  }
  return recs;
}

function lineStarts(text) {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === "\n") starts.push(i + 1);
  return starts;
}

// 🧑 "clicking on the preview would teleport. The editor cursor at exactly the correct spot, as opposed to teleporting nearby."
/** PDF point (`x`,`y` from the top-left of 1-based `page`) to `{file, line, col}` (0-based).
 * `before`/`after` are the rendered letters around the click; `readText(file)` gives source. */
export async function inverse({ pdf, page, x, y, before, after }, readText) {
  const [r] = await synctex(["edit", "-o", `${page}:${x}:${y}:${pdf}`]);
  if (!r?.Input) return null;
  const file = path.normalize(r.Input);
  const line = Math.max(0, Number(r.Line) - 1);
  const text = readText(file);
  if (text == null) return { file, line, col: 0 };
  const starts = lineStarts(text);
  const want = before.length + after.length;
  const near = (lo, hi) => {
    const win = letters(text.slice(lo, hi), true);
    const lineLo = starts[Math.min(line, starts.length - 1)] - lo;
    const prefer = Math.max(0, win.idx.findIndex((i) => i >= lineLo));
    return { ...bestSplit(win.str, before, after, prefer), win, lo };
  };
  let m = near(starts[Math.max(0, line - WINDOW_LINES)], starts[line + WINDOW_LINES + 1] ?? text.length);
  if (m.score < MIN_FIT * want) {
    const all = near(0, text.length);
    if (all.score > m.score) m = all;
  }
  if (m.score <= 0) return { file, line, col: 0 };
  const off = m.lo + (m.pos < m.win.idx.length ? m.win.idx[m.pos] : m.win.idx.at(-1) + 1);
  const l = starts.findLastIndex((s) => s <= off);
  return { file, line: l, col: off - starts[l] };
}

/** Source position (0-based `line`,`col` in `file`) to a PDF box plus the source letters
 * around it, which the viewer matches against the page text for the exact spot. */
export async function forward({ pdf, file, line, col }, readText) {
  const recs = await synctex(["view", "-i", `${line + 1}:${col + 1}:${file}`, "-o", pdf]);
  if (!recs.length) return null;
  const r = recs[0];
  const text = readText(file) ?? "";
  const starts = lineStarts(text);
  const off = (starts[line] ?? text.length) + col;
  return {
    page: Number(r.Page),
    box: { x: Number(r.h), y: Number(r.v) - Number(r.H), w: Number(r.W), h: Number(r.H) },
    before: letters(text.slice(Math.max(0, off - 4 * CONTEXT_CHARS), off), true).str.slice(-CONTEXT_CHARS),
    after: letters(text.slice(off, off + 4 * CONTEXT_CHARS), true).str.slice(0, CONTEXT_CHARS),
  };
}
