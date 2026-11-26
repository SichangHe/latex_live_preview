/** Locates a clicked spot of rendered text inside LaTeX source (and back) by matching the
 * letters around it; shared by the server and the browser viewer. See docs/sync.md. */

/** Letters and digits of `text`, lowercased and with ligatures/accents decomposed, plus each
 * kept char's index in `text`. With `tex`, control-sequence names and comments are skipped. */
export function letters(text, tex) {
  const chars = [];
  const idx = [];
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (tex && c === "%") {
      const nl = text.indexOf("\n", i);
      if (nl < 0) break;
      i = nl;
      continue;
    }
    if (tex && c === "\\") {
      let j = i + 1;
      while (j < text.length && /[A-Za-z@]/.test(text[j])) j++;
      i = j === i + 1 ? i + 1 : j - 1;
      continue;
    }
    for (const d of c.normalize("NFKD").toLowerCase()) {
      if (/[\p{L}\p{N}]/u.test(d)) {
        chars.push(d);
        idx.push(i);
      }
    }
  }
  return { str: chars.join(""), idx };
}

/** Position in `hay` (a `letters` string) that best fits `before`|`after` (letters strings),
 * scored by the matched suffix of `before` plus matched prefix of `after`. Ties go to the
 * position nearest `prefer`. Returns `{pos, score}`. */
export function bestSplit(hay, before, after, prefer) {
  let best = { pos: prefer, score: -1 };
  for (let p = 0; p <= hay.length; p++) {
    let a = 0;
    while (a < before.length && p - a - 1 >= 0 && hay[p - a - 1] === before[before.length - a - 1]) a++;
    let b = 0;
    while (b < after.length && p + b < hay.length && hay[p + b] === after[b]) b++;
    const score = a + b;
    if (score > best.score || (score === best.score && Math.abs(p - prefer) < Math.abs(best.pos - prefer))) {
      best = { pos: p, score };
    }
  }
  return best;
}
