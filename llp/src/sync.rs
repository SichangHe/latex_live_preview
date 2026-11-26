//! SyncTeX queries refined to exact characters by letter matching; see `docs/sync.md`.
//! `letters` and `best_split` mirror `viewer/match.js`.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use unicode_normalization::UnicodeNormalization;

const WINDOW_LINES: usize = 6;
const CONTEXT_CHARS: usize = 40;
/// Below this fraction of context letters matched near the SyncTeX line, search the file.
const MIN_FIT: f64 = 0.8;

/// Letters and digits of `text`, lowercased and decomposed, with each one's byte offset.
/// With `tex`, control-sequence names and comments are skipped.
pub fn letters(text: &str, tex: bool) -> (Vec<char>, Vec<usize>) {
    let (mut chars, mut idx) = (Vec::new(), Vec::new());
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if tex && c == '%' {
            if !it.by_ref().any(|(_, c)| c == '\n') {
                break;
            }
            continue;
        }
        if tex && c == '\\' {
            let mut named = false;
            while it
                .next_if(|(_, c)| c.is_ascii_alphabetic() || *c == '@')
                .is_some()
            {
                named = true;
            }
            if !named {
                it.next();
            }
            continue;
        }
        for d in c
            .nfkd()
            .flat_map(char::to_lowercase)
            .filter(|d| d.is_alphanumeric())
        {
            chars.push(d);
            idx.push(i);
        }
    }
    (chars, idx)
}

/// Position in `hay` best fitting `before`|`after`, scored by matched suffix of `before` plus
/// matched prefix of `after`; ties go to the position nearest `prefer`. Returns (pos, score).
pub fn best_split(hay: &[char], before: &[char], after: &[char], prefer: usize) -> (usize, usize) {
    let mut best = (prefer, 0, false);
    for p in 0..=hay.len() {
        let a = before
            .iter()
            .rev()
            .zip(hay[..p].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let b = after
            .iter()
            .zip(&hay[p..])
            .take_while(|(x, y)| x == y)
            .count();
        let score = a + b;
        if !best.2
            || score > best.1
            || (score == best.1 && p.abs_diff(prefer) < best.0.abs_diff(prefer))
        {
            best = (p, score, true);
        }
    }
    (best.0, best.1)
}

/// Byte offset of each line start.
pub fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// LSP position (UTF-16 column) to byte offset, clamped to the text.
pub fn pos2byte(text: &str, starts: &[usize], line: usize, col16: usize) -> usize {
    let Some(&s) = starts.get(line) else {
        return text.len();
    };
    let mut units = 0;
    for (i, c) in text[s..].char_indices() {
        if units >= col16 || c == '\n' {
            return s + i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// Byte offset to LSP (line, UTF-16 column).
pub fn byte2pos(text: &str, starts: &[usize], off: usize) -> (usize, usize) {
    let line = starts.partition_point(|&s| s <= off).saturating_sub(1);
    (line, text[starts[line]..off].encode_utf16().count())
}

/// Runs `synctex` and splits its output into records, each starting at an `Output:` line.
fn synctex(args: &[&str]) -> Vec<Vec<(String, String)>> {
    let Ok(out) = Command::new("synctex").args(args).output() else {
        return vec![];
    };
    let mut recs: Vec<Vec<(String, String)>> = Vec::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        match recs.last_mut() {
            _ if k == "Output" => recs.push(vec![]),
            Some(r) => r.push((k.into(), v.into())),
            None => {}
        }
    }
    recs
}

fn field<'a>(rec: &'a [(String, String)], k: &str) -> Option<&'a str> {
    rec.iter()
        .find(|(key, _)| key == k)
        .map(|(_, v)| v.as_str())
}

pub struct Click {
    pub pdf: PathBuf,
    pub page: u64,
    pub x: f64,
    pub y: f64,
    pub before: Vec<char>,
    pub after: Vec<char>,
}

// 🧑 "clicking on the preview would teleport. The editor cursor at exactly the correct spot, as opposed to teleporting nearby."
/// PDF point to (file, line, UTF-16 column); `read_text(file)` gives the current source.
pub fn inverse(
    c: &Click,
    read_text: impl Fn(&Path) -> Option<String>,
) -> Option<(PathBuf, usize, usize)> {
    let recs = synctex(&[
        "edit",
        "-o",
        &format!("{}:{}:{}:{}", c.page, c.x, c.y, c.pdf.display()),
    ]);
    let rec = recs.first()?;
    let file = PathBuf::from(field(rec, "Input")?);
    let file = file.canonicalize().unwrap_or(file);
    let line = field(rec, "Line")?.parse::<usize>().ok()?.saturating_sub(1);
    let Some(text) = read_text(&file) else {
        return Some((file, line, 0));
    };
    let starts = line_starts(&text);
    let line = line.min(starts.len() - 1);
    let near = |lo: usize, hi: usize| {
        let (hay, idx) = letters(&text[lo..hi], true);
        let prefer = idx
            .iter()
            .position(|&i| lo + i >= starts[line])
            .unwrap_or(idx.len());
        let (pos, score) = best_split(&hay, &c.before, &c.after, prefer);
        let off = idx
            .get(pos)
            .map(|i| lo + i)
            .or_else(|| idx.last().map(|i| lo + i + 1));
        (score, off)
    };
    let hi = starts
        .get(line + WINDOW_LINES + 1)
        .copied()
        .unwrap_or(text.len());
    let mut m = near(starts[line.saturating_sub(WINDOW_LINES)], hi);
    if (m.0 as f64) < MIN_FIT * (c.before.len() + c.after.len()) as f64 {
        let all = near(0, text.len());
        if all.0 > m.0 {
            m = all;
        }
    }
    match m {
        (score, Some(off)) if score > 0 => {
            let (l, col) = byte2pos(&text, &starts, off);
            Some((file, l, col))
        }
        _ => Some((file, line, 0)),
    }
}

/// Source position to a PDF box plus the source letters around it, which the viewer matches
/// against the page text for the exact spot.
pub fn forward(pdf: &Path, file: &Path, line: usize, col16: usize, text: &str) -> Option<Value> {
    let recs = synctex(&[
        "view",
        "-i",
        &format!("{}:{}:{}", line + 1, col16 + 1, file.display()),
        "-o",
        &pdf.display().to_string(),
    ]);
    let rec = recs.first()?;
    let num = |k: &str| {
        field(rec, k)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let starts = line_starts(text);
    let off = pos2byte(text, &starts, line, col16);
    let lo = text.floor_char_boundary(off.saturating_sub(4 * CONTEXT_CHARS));
    let hi = text.ceil_char_boundary((off + 4 * CONTEXT_CHARS).min(text.len()));
    let before: String = letters(&text[lo..off], true)
        .0
        .into_iter()
        .rev()
        .take(CONTEXT_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = letters(&text[off..hi], true)
        .0
        .into_iter()
        .take(CONTEXT_CHARS)
        .collect();
    Some(json!({
        "page": num("Page"),
        "box": { "x": num("h"), "y": num("v") - num("H"), "w": num("W"), "h": num("H") },
        "before": before,
        "after": after,
    }))
}
