//! Project directory watching by polling, so changes made outside the editor recompile. Polling
//! is portable (Linux, macOS) and copes with new directories and atomic renames alike.
use crate::Msg;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, SystemTime};

const POLL: Duration = Duration::from_millis(300);
const MAX_DIRS: usize = 512;
const MAX_DEPTH: usize = 4;

/// Modification time and size of every non-hidden file under `root` passing `want`.
fn scan(root: &Path, want: fn(&Path) -> bool) -> HashMap<PathBuf, (SystemTime, u64)> {
    let mut files = HashMap::new();
    let (mut stack, mut n_dirs) = (vec![(root.to_path_buf(), 0)], 0);
    while let Some((dir, depth)) = stack.pop() {
        n_dirs += 1;
        if n_dirs > MAX_DIRS {
            break;
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if e.file_name().as_encoded_bytes().starts_with(b".") {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((e.path(), depth + 1));
                }
            } else if want(&e.path()) {
                files.insert(
                    e.path(),
                    (
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        meta.len(),
                    ),
                );
            }
        }
    }
    files
}

/// Sends `Msg::Fs(path)` for each file under `root` passing `want` that appears or changes,
/// and for each changing file received on `extra` (inputs outside `root`, e.g. `../x.tex`).
pub fn spawn(root: PathBuf, want: fn(&Path) -> bool, extra: Receiver<PathBuf>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let mut extras: Vec<PathBuf> = vec![];
        let stamp = |p: &Path| {
            std::fs::metadata(p)
                .ok()
                .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
        };
        let scan_all = |extras: &[PathBuf]| {
            let mut files = scan(&root, want);
            files.extend(extras.iter().filter_map(|p| Some((p.clone(), stamp(p)?))));
            files
        };
        let mut seen = scan_all(&extras);
        loop {
            std::thread::sleep(POLL);
            for p in extra.try_iter() {
                if let Some(s) = stamp(&p) {
                    seen.insert(p.clone(), s);
                }
                extras.push(p);
            }
            let now = scan_all(&extras);
            for (path, stamp) in &now {
                if seen.get(path) != Some(stamp) && tx.send(Msg::Fs(path.clone())).is_err() {
                    return;
                }
            }
            seen = now;
        }
    });
}
