/** Browser viewer: composes the newest partial PDF over the last complete one, renders only
 * visible pages that changed, and maps clicks/cursor to exact characters; see docs/viewer.md. */
import * as pdfjs from "/pdf.mjs";
import { bestSplit, letters } from "/match.js";

pdfjs.GlobalWorkerOptions.workerSrc = "/pdf.worker.mjs";
/** One worker for all documents; a worker per document costs ~80 ms to start. */
const worker = new pdfjs.PDFWorker();
const CONTEXT_CHARS = 40;
const pagesEl = document.getElementById("pages");
const statusEl = document.getElementById("status");
/** kind (`done`|`partial`) -> {gen, url, doc, nPages} */
const docs = {};
/** 1-based page -> {el, dirty, gen, size, zoom}; `gen` is the generation it shows; index 0 unused */
const pages = [null];
let defaultSize = { w: 612, h: 792 };
/** `+`/`-` scale pages relative to fitting the window width; `0` resets. */
let userZoom = 1;
const zoom = () => userZoom * Math.min(2, (innerWidth - 24) / defaultSize.w);

/** Which loaded PDF shows page `i`: the partial if it is newer and has the page. */
function sourceFor(i) {
  const { done, partial } = docs;
  if (partial && (!done || partial.gen > done.gen) && i <= partial.nPages) return partial;
  if (done && i <= done.nPages) return done;
  return partial && i <= partial.nPages ? partial : null;
}

const visible = new Set();
const observer = new IntersectionObserver((entries) => {
  for (const e of entries) {
    const i = Number(e.target.dataset.page);
    if (e.isIntersecting) visible.add(i);
    else visible.delete(i);
  }
  renderVisible();
}, { rootMargin: "100% 0px" });

function layout() {
  const n = Math.max(docs.done?.nPages ?? 0, docs.partial && (!docs.done || docs.partial.gen > docs.done.gen) ? docs.partial.nPages : 0);
  while (pages.length - 1 < n) {
    const el = document.createElement("div");
    el.className = "page";
    el.dataset.page = pages.length;
    pages.push({ el, dirty: true, gen: null, size: null });
    pagesEl.append(el);
    observer.observe(el);
  }
  while (pages.length - 1 > n) {
    const p = pages.pop();
    observer.unobserve(p.el);
    p.el.remove();
    visible.delete(pages.length);
  }
  const z = zoom();
  for (let i = 1; i < pages.length; i++) {
    const s = pages[i].size ?? defaultSize;
    Object.assign(pages[i].el.style, { width: `${s.w * z}px`, height: `${s.h * z}px` });
  }
}

/** Renders dirty pages near the viewport, the most central first. */
let rendering = Promise.resolve();
function renderVisible() {
  rendering = rendering.then(async () => {
    const dist = (i) => Math.abs(pages[i].el.getBoundingClientRect().top + pages[i].el.offsetHeight / 2 - innerHeight / 2);
    for (const i of [...visible].filter((i) => pages[i]?.dirty).sort((a, b) => dist(a) - dist(b))) await renderPage(i);
  }).catch((e) => console.warn(e));
  return rendering;
}

async function renderPage(i) {
  const src = sourceFor(i);
  const p = pages[i];
  if (!src || !p?.dirty) return;
  const page = await src.doc.getPage(i);
  const z = zoom();
  const vp = page.getViewport({ scale: z });
  p.size = { w: vp.width / z, h: vp.height / z };
  const canvas = document.createElement("canvas");
  const dpr = devicePixelRatio || 1;
  canvas.width = Math.round(vp.width * dpr);
  canvas.height = Math.round(vp.height * dpr);
  await page.render({ canvasContext: canvas.getContext("2d"), viewport: vp, transform: [dpr, 0, 0, dpr, 0, 0] }).promise;
  const text = document.createElement("div");
  text.className = "textLayer";
  text.style.setProperty("--scale-factor", z);
  await new pdfjs.TextLayer({ textContentSource: await page.getTextContent(), container: text, viewport: vp }).render();
  Object.assign(p.el.style, { width: `${vp.width}px`, height: `${vp.height}px` });
  p.el.replaceChildren(canvas, text);
  p.dirty = false;
  p.gen = src.gen;
  p.zoom = z;
}

async function load(msg) {
  const doc = await pdfjs.getDocument({ url: msg.url, isEvalSupported: false, worker }).promise.catch(() => null);
  if (!doc) return;
  const cur = docs[msg.kind];
  if (cur && cur.gen > msg.gen) return doc.destroy();
  if (msg.kind === "partial" && cur?.gen === msg.gen && cur.nPages > doc.numPages) return doc.destroy();
  const old = docs[msg.kind];
  docs[msg.kind] = { gen: msg.gen, url: msg.url, doc, nPages: doc.numPages };
  if (pages.length <= 1) {
    const vp = (await doc.getPage(1)).getViewport({ scale: 1 });
    defaultSize = { w: vp.width, h: vp.height };
  }
  layout();
  for (let i = msg.firstChanged; i <= doc.numPages && i < pages.length; i++) {
    if (pages[i].gen !== msg.gen && sourceFor(i)?.gen === msg.gen) pages[i].dirty = true;
  }
  await renderVisible();
  old?.doc.destroy();
  statusEl.textContent = `gen ${msg.gen} ${msg.kind}`;
}

/** Page text from the text layer, with each span's start offset. */
function pageText(el) {
  const spans = [...el.querySelectorAll(".textLayer span:not(:has(span))")];
  let text = "";
  const starts = [];
  for (const s of spans) {
    starts.push(text.length);
    text += s.textContent;
  }
  return { spans, starts, text };
}

function onClick(ev) {
  if (!getSelection().isCollapsed) return;
  const el = ev.target.closest(".page");
  if (!el) return;
  const i = Number(el.dataset.page);
  const src = sourceFor(i);
  const rect = el.getBoundingClientRect();
  const z = pages[i].zoom ?? zoom();
  let before = "";
  let after = "";
  const caret = document.caretPositionFromPoint?.(ev.clientX, ev.clientY);
  const node = caret?.offsetNode;
  const span = node?.parentElement?.closest(".textLayer span");
  if (span && el.contains(span)) {
    const { spans, starts, text } = pageText(el);
    const k = spans.indexOf(span);
    if (k >= 0) {
      const g = starts[k] + caret.offset;
      before = letters(text.slice(0, g)).str.slice(-CONTEXT_CHARS);
      after = letters(text.slice(g)).str.slice(0, CONTEXT_CHARS);
    }
  }
  fetch("/click", {
    method: "POST",
    body: JSON.stringify({ url: src.url, page: i, x: (ev.clientX - rect.left) / z, y: (ev.clientY - rect.top) / z, before, after }),
  });
}

/** Scrolls to and marks the exact character matching the source context. */
async function reveal({ page, box, before, after }) {
  const p = pages[page];
  if (!p) return;
  const z = () => p.zoom ?? zoom();
  const boxPx = () => ({ left: box.x * z(), top: box.y * z(), width: box.w * z(), height: Math.max(box.h, 8) * z() });
  const r = p.el.getBoundingClientRect();
  const b = boxPx();
  if (r.top + b.top < 0 || r.top + b.top + b.height > innerHeight) scrollBy({ top: r.top + b.top - innerHeight / 3 });
  visible.add(page);
  await renderVisible();
  for (const m of p.el.querySelectorAll(".marker")) m.remove();
  const marker = document.createElement("div");
  const { spans, starts, text } = pageText(p.el);
  const lt = letters(text);
  const pr = p.el.getBoundingClientRect();
  const near = spans.findIndex((s) => {
    const sr = s.getBoundingClientRect();
    return sr.bottom - pr.top >= b.top && sr.top - pr.top <= b.top + b.height;
  });
  const prefer = near < 0 ? 0 : Math.max(0, lt.idx.findIndex((x) => x >= starts[near]));
  const m = (before || after) && lt.str ? bestSplit(lt.str, before, after, prefer) : { score: 0 };
  if (m.score > 0) {
    const g = m.pos < lt.idx.length ? lt.idx[m.pos] : text.length;
    const k = starts.findLastIndex((s) => s <= g);
    const range = document.createRange();
    const tn = spans[k].firstChild;
    range.setStart(tn, Math.min(g - starts[k], tn.length));
    const cr = range.getClientRects()[0] ?? spans[k].getBoundingClientRect();
    marker.className = "marker caret";
    Object.assign(marker.style, { left: `${cr.left - pr.left}px`, top: `${cr.top - pr.top}px`, height: `${cr.height}px` });
  } else {
    marker.className = "marker box";
    Object.assign(marker.style, Object.fromEntries(Object.entries(boxPx()).map(([k, v]) => [k, `${v}px`])));
  }
  p.el.append(marker);
}

document.addEventListener("click", onClick);
addEventListener("keydown", (e) => {
  const z = { "+": userZoom * 1.2, "=": userZoom * 1.2, "-": userZoom / 1.2, 0: 1 }[e.key];
  if (z === undefined || e.ctrlKey || e.metaKey) return;
  userZoom = z;
  dispatchEvent(new Event("resize"));
});
addEventListener("resize", () => {
  for (const p of pages.slice(1)) p.dirty = true;
  layout();
  renderVisible();
});
const es = new EventSource("/events");
es.onopen = () => (statusEl.textContent = "connected");
es.onerror = () => (statusEl.textContent = "disconnected");
es.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.type === "pdf") load(msg);
  else if (msg.type === "reveal") reveal(msg);
};
