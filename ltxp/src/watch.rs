//! Project directory watching with inotify, so changes made outside the editor recompile.
use crate::Msg;
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

const MAX_DIRS: usize = 512;
const MAX_DEPTH: usize = 4;

pub struct Watcher {
    fd: i32,
    dirs: HashMap<i32, PathBuf>,
}

impl Watcher {
    /// Watches `root` and its non-hidden subdirectories; events go to `tx` as `Msg::Fs`.
    pub fn new(root: &Path, tx: Sender<Msg>) -> Option<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        std::thread::spawn(move || read_events(fd, tx));
        let mut w = Self {
            fd,
            dirs: HashMap::new(),
        };
        let mut stack = vec![(root.to_path_buf(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            if w.dirs.len() >= MAX_DIRS {
                break;
            }
            w.add(&dir);
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let hidden = e.file_name().as_bytes().starts_with(b".");
                if depth < MAX_DEPTH && !hidden && e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push((e.path(), depth + 1));
                }
            }
        }
        Some(w)
    }

    fn add(&mut self, dir: &Path) {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else {
            return;
        };
        let wd = unsafe {
            libc::inotify_add_watch(
                self.fd,
                c.as_ptr(),
                libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE,
            )
        };
        if wd >= 0 {
            self.dirs.insert(wd, dir.to_path_buf());
        }
    }

    /// Path of a file event from the reader thread.
    pub fn path(&self, wd: i32, name: &OsStr) -> Option<PathBuf> {
        Some(self.dirs.get(&wd)?.join(name))
    }

    /// Starts watching a directory created after startup.
    pub fn add_new(&mut self, dir: &Path) {
        if self.dirs.len() < MAX_DIRS
            && !dir
                .file_name()
                .is_some_and(|n| n.as_bytes().starts_with(b"."))
        {
            self.add(dir);
        }
    }
}

fn read_events(fd: i32, tx: Sender<Msg>) {
    let mut buf = vec![0u8; 64 * 1024];
    let head = std::mem::size_of::<libc::inotify_event>();
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return;
        }
        let mut off = 0;
        while off + head <= n as usize {
            let ev: libc::inotify_event =
                unsafe { std::ptr::read_unaligned(buf[off..].as_ptr().cast()) };
            let name = &buf[off + head..off + head + ev.len as usize];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
            if !name.is_empty()
                && tx
                    .send(Msg::Fs(
                        ev.wd,
                        OsStr::from_bytes(name).to_os_string(),
                        ev.mask & libc::IN_ISDIR != 0,
                    ))
                    .is_err()
            {
                return;
            }
            off += head + ev.len as usize;
        }
    }
}
