//! LuaLaTeX generations and their snapshots; see `docs/engine.md`.
use crate::Msg;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Files a run writes that are not inputs of the next run.
const NOT_SEED: [&str; 3] = ["main.pdf", "main.synctex", "main.log"];
/// No `run` event within this after `go` means the snapshot is dead.
const WATCHDOG: Duration = Duration::from_secs(3);
const DRIVER: &str = include_str!("../../engine/driver.lua");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Partial,
    Done,
}

/// A complete PDF of generation `generation`: pages before `first_changed` equal the last delivered
/// done PDF's; `seed` hashes the aux files its lineage started from.
#[derive(Clone, Debug)]
pub struct Pdf {
    pub generation: u64,
    pub kind: Kind,
    pub first_changed: u64,
    pub dir: PathBuf,
    pub seed: u64,
}

struct Snap {
    pid: i32,
    max_read: HashMap<PathBuf, usize>,
    generation: u64,
}

pub struct Engine {
    main_tex: PathBuf,
    run_dir: PathBuf,
    native: PathBuf,
    tx: Sender<Msg>,
    snaps: BTreeMap<u64, Snap>,
    /// each parked snapshot costs ~5 MB; beyond `max_snaps`, keep pages divisible by `stride`
    max_snaps: usize,
    stride: u64,
    generation: u64,
    first_changed: u64,
    /// a done PDF was delivered since `first_changed` was last reset
    delivered: bool,
    seed_of: HashMap<u64, u64>,
    /// pids running the current generation
    running: Vec<i32>,
    /// process groups of cold gens; every snapshot, clone and finisher is inside one
    groups: Vec<i32>,
    /// resumed generation and its snapshot page, until the clone reports
    awaiting: Option<(u64, u64)>,
    /// canonical path of each raw path TeX reported
    canonical: HashMap<String, PathBuf>,
    new_inputs: Vec<PathBuf>,
}

/// Hash of the aux files (`.aux`, `.bbl`, `.toc`, ...) in `dir`.
pub fn seed_sig(dir: &Path) -> u64 {
    let mut names: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    names.sort();
    let mut h = DefaultHasher::new();
    for n in names.iter().filter(|n| {
        !NOT_SEED
            .iter()
            .any(|s| n.as_encoded_bytes() == s.as_bytes())
    }) {
        n.hash(&mut h);
        fs::read(dir.join(n)).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

fn kill(pid: i32) {
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

impl Engine {
    /// Prepares `run_dir` (event FIFO, shadow dir, driver) and starts reading events into `tx`.
    pub fn new(
        main_tex: PathBuf,
        run_dir: PathBuf,
        native: PathBuf,
        tx: Sender<Msg>,
    ) -> std::io::Result<Self> {
        fs::create_dir_all(run_dir.join("shadow"))?;
        fs::write(run_dir.join("driver.lua"), DRIVER)?;
        let ev = run_dir.join("ev");
        let c = std::ffi::CString::new(ev.as_os_str().as_encoded_bytes())
            .map_err(std::io::Error::other)?;
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let f = fs::OpenOptions::new().read(true).write(true).open(&ev)?;
        let etx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(f).lines().map_while(Result::ok) {
                if etx.send(Msg::Ev(line)).is_err() {
                    return;
                }
            }
        });
        let max_snaps = std::env::var("LTXP_MAX_SNAPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        Ok(Self {
            main_tex,
            run_dir,
            native,
            tx,
            snaps: BTreeMap::new(),
            max_snaps,
            stride: 1,
            generation: 0,
            first_changed: 1,
            delivered: true,
            seed_of: HashMap::new(),
            running: vec![],
            groups: vec![],
            awaiting: None,
            canonical: HashMap::new(),
            new_inputs: vec![],
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether TeX reads `file`, per the snapshots.
    pub fn reads(&self, file: &Path) -> bool {
        self.snaps.values().any(|s| s.max_read.contains_key(file))
    }

    /// A generation is compiling.
    pub fn busy(&self) -> bool {
        !self.running.is_empty() || self.awaiting.is_some()
    }

    /// Input files first reported since the last call.
    pub fn take_new_inputs(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.new_inputs)
    }

    fn tell_snap(&self, pid: i32, cmd: &str) {
        let fifo = self.run_dir.join(format!("snap-{pid}"));
        if let Ok(mut f) = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
        {
            let _ = writeln!(f, "{cmd}");
        }
        if cmd == "quit" {
            let _ = fs::remove_file(fifo);
        }
    }

    fn drop_snap(&mut self, page: u64) {
        if let Some(s) = self.snaps.remove(&page) {
            self.tell_snap(s.pid, "quit");
        }
    }

    fn kill_running(&mut self) {
        self.running.drain(..).for_each(kill);
    }

    /// Pages from `page` on may differ from what the viewer last got complete.
    fn mark_changed(&mut self, page: u64) {
        self.first_changed = if self.delivered {
            page
        } else {
            self.first_changed.min(page)
        };
        self.delivered = false;
    }

    /// Compiles from scratch, starting from the aux files in `seed_dir` if given. Existing
    /// snapshots stay usable until the new run replaces them.
    pub fn cold(&mut self, seed_dir: Option<&Path>) {
        self.kill_running();
        self.awaiting = None;
        self.generation += 1;
        self.mark_changed(1);
        if seed_dir.is_none() {
            self.stride = 1;
        }
        self.seed_of
            .insert(self.generation, seed_dir.map_or(0, seed_sig));
        let dir = self.run_dir.join(format!("g{}", self.generation));
        if let Err(e) = fs::create_dir(&dir) {
            return crate::log(&format!("gen {}: {e}", self.generation));
        }
        for e in seed_dir
            .and_then(|d| fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
        {
            if !NOT_SEED
                .iter()
                .any(|s| e.file_name().as_encoded_bytes() == s.as_bytes())
            {
                let _ = fs::copy(e.path(), dir.join(e.file_name()));
            }
        }
        let mut spawned = Command::new("bash");
        spawned
            .args(["-c", r#"exec 200<"$LTXP_OUT" && exec lualatex --shell-escape -synctex=-1 -interaction=nonstopmode -file-line-error -jobname=main -output-directory=/proc/self/fd/200 '\directlua{dofile(os.getenv("LTXP_DRIVER"))}'"#])
            .current_dir(self.main_tex.parent().unwrap_or(Path::new("/")))
            .env("LTXP_NATIVE", &self.native)
            .env("LTXP_EVENTS", self.run_dir.join("ev"))
            .env("LTXP_RUN", &self.run_dir)
            .env("LTXP_SHADOW", self.run_dir.join("shadow"))
            .env("LTXP_DRIVER", self.run_dir.join("driver.lua"))
            .env("LTXP_MAIN", &self.main_tex)
            .env("LTXP_GEN", self.generation.to_string())
            .env("LTXP_OUT", &dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let spawned = spawned.spawn();
        match spawned {
            Ok(child) => {
                let pid = child.id() as i32;
                self.groups.push(pid);
                self.running = vec![pid];
                std::thread::spawn(move || drop(child.wait_with_output()));
                crate::log(&format!("gen {}: cold start", self.generation));
            }
            Err(e) => crate::log(&format!(
                "gen {}: cannot start lualatex: {e}",
                self.generation
            )),
        }
    }

    // 🧑 "This also needs to work for very long documents. So you need incremental change support."
    /// Recompiles after `file` changed from 1-based `line` on: resumes the latest snapshot that
    /// has not read that line; snapshots that have are dropped.
    pub fn edit(&mut self, file: &Path, line: usize) {
        let stale: Vec<u64> = self
            .snaps
            .iter()
            .filter(|(_, s)| s.max_read.get(file).is_some_and(|&n| n >= line))
            .map(|(&p, _)| p)
            .collect();
        let best = self
            .snaps
            .keys()
            .rev()
            .find(|p| !stale.contains(p))
            .copied();
        let Some(best) = best else {
            stale.into_iter().for_each(|p| self.drop_snap(p));
            return self.cold(None);
        };
        self.kill_running();
        self.generation += 1;
        self.mark_changed(best + 1);
        let snap = &self.snaps[&best];
        let seed = self.seed_of.get(&snap.generation).copied().unwrap_or(0);
        self.seed_of.insert(self.generation, seed);
        self.tell_snap(snap.pid, &format!("go {}", self.generation));
        stale.into_iter().for_each(|p| self.drop_snap(p));
        self.awaiting = Some((self.generation, best));
        let (tx, generation) = (self.tx.clone(), self.generation);
        std::thread::spawn(move || {
            std::thread::sleep(WATCHDOG);
            let _ = tx.send(Msg::Watchdog(generation));
        });
        crate::log(&format!(
            "gen {}: resume after page {best}",
            self.generation
        ));
    }

    pub fn on_watchdog(&mut self, generation: u64) {
        if let Some((g, page)) = self.awaiting
            && g == generation
            && self.generation == generation
        {
            crate::log(&format!(
                "gen {generation}: snapshot of page {page} did not resume; cold start"
            ));
            self.snaps.remove(&page);
            self.cold(None);
        }
    }

    fn shadow_path(&self, file: &Path) -> PathBuf {
        self.run_dir
            .join("shadow")
            .join(file.strip_prefix("/").unwrap_or(file))
    }

    /// Mirrors an unsaved editor buffer so TeX reads it instead of the file on disk.
    pub fn write_shadow(&self, file: &Path, text: &str) {
        let p = self.shadow_path(file);
        if let Some(d) = p.parent() {
            let _ = fs::create_dir_all(d);
        }
        let _ = fs::write(p, text);
    }

    pub fn drop_shadow(&self, file: &Path) {
        let _ = fs::remove_file(self.shadow_path(file));
    }

    fn canonical(&mut self, name: &str) -> PathBuf {
        if let Some(p) = self.canonical.get(name) {
            return p.clone();
        }
        let p = fs::canonicalize(name).unwrap_or_else(|_| PathBuf::from(name));
        self.canonical.insert(name.into(), p.clone());
        self.new_inputs.push(p.clone());
        p
    }

    /// Handles one driver event line `GEN KIND ...`; returns a PDF that became complete.
    pub fn on_event(&mut self, line: &str) -> Option<Pdf> {
        let mut parts = line.splitn(3, ' ');
        let (g, kind, rest) = (
            parts.next()?.parse::<u64>().ok()?,
            parts.next()?,
            parts.next().unwrap_or(""),
        );
        let mut args = rest.split(' ');
        if g != self.generation {
            match kind {
                "snap" => self.tell_snap(args.nth(1)?.parse().ok()?, "quit"),
                "run" => kill(args.nth(1)?.parse().ok()?),
                _ => {}
            }
            return None;
        }
        let pdf = |kind, dir: &str| {
            Some(Pdf {
                generation: g,
                kind,
                first_changed: self.first_changed,
                dir: dir.into(),
                seed: self.seed_of.get(&g).copied().unwrap_or(0),
            })
        };
        match kind {
            "snap" => {
                let (page, pid) = (
                    args.next()?.parse::<u64>().ok()?,
                    args.next()?.parse::<i32>().ok()?,
                );
                if page % self.stride != 0 {
                    self.tell_snap(pid, "quit");
                    return None;
                }
                self.drop_snap(page);
                let mr = rest.splitn(3, ' ').nth(2).unwrap_or("");
                let max_read = mr
                    .split('\t')
                    .filter_map(|kv| kv.rsplit_once(':'))
                    .filter(|(f, _)| !f.starts_with("/proc/"))
                    .filter_map(|(f, n)| Some((self.canonical(f), n.parse().ok()?)))
                    .collect();
                self.snaps.insert(
                    page,
                    Snap {
                        pid,
                        max_read,
                        generation: g,
                    },
                );
                if self.snaps.len() > self.max_snaps {
                    self.stride *= 2;
                    let odd: Vec<u64> = self
                        .snaps
                        .keys()
                        .filter(|p| *p % self.stride != 0)
                        .copied()
                        .collect();
                    odd.into_iter().for_each(|p| self.drop_snap(p));
                }
                None
            }
            "run" => {
                self.awaiting = None;
                self.running.push(args.nth(1)?.parse().ok()?);
                None
            }
            "partial" => pdf(Kind::Partial, args.nth(1)?),
            "done" => {
                let p = pdf(Kind::Done, args.next()?);
                self.delivered = true;
                self.running.clear();
                p
            }
            "error" => {
                crate::log(&format!("gen {g} error: {rest}"));
                None
            }
            _ => None,
        }
    }

    /// Kills every process this engine started, directly or by forking.
    pub fn close(&mut self) {
        for g in self.groups.drain(..) {
            kill(-g);
        }
    }
}
