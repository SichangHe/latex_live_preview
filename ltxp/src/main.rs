// 🧑 "Layout using. Latex engine, but display using a web browser. Doing some sort of. Language server or editor plugin. Ideally, mainly language server."
//! `ltxp`: instant LaTeX preview language server with a browser viewer; see `docs/index.md`.
//! One actor thread (`Server`) owns all state; stdin, the engine's event FIFO, HTTP
//! connections, inotify, timers and SyncTeX workers send it `Msg`s.
mod engine;
mod http;
mod lsp;
mod sync;
mod uninstall;
mod watch;

use engine::{Engine, Kind, Pdf, seed_sig};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, SystemTime};

/// Like latexmk's default `$max_repeat`; reruns stop earlier once aux files are stable.
const MAX_AUTO_RERUNS: u32 = 5;
/// How many parent folders to search for the main file of a sub-file.
const MAIN_SEARCH_UP: usize = 4;
/// Stop the main-file search after this many `.tex` files.
const MAIN_SEARCH_FILES: usize = 2000;
const GC_INTERVAL: Duration = Duration::from_secs(10);
/// Extensions of files whose change on disk recompiles.
const WATCHED_EXTS: [&str; 20] = [
    "tex", "sty", "cls", "clo", "cfg", "def", "fd", "lua", "bib", "bst", "bbx", "cbx", "png",
    "jpg", "jpeg", "pdf", "eps", "svg", "csv", "tikz",
];
const USAGE: &str = "ltxp: instant LaTeX preview in the browser

usage:
  ltxp            run as a language server on stdin/stdout (for editors)
  ltxp FILE.tex   preview FILE.tex without an editor, recompiling on file changes
  ltxp uninstall [--yes]   remove everything the installer set up (asks first)
  ltxp --version

env: LTXP_PORT (viewer port, default random), LTXP_MAX_SNAPS (default 400)";

pub enum Msg {
    Lsp(Value),
    Quit,
    Ev(String),
    Sse(TcpStream),
    Click(Value),
    /// a project file appeared or changed on disk
    Fs(PathBuf),
    Tick,
    Watchdog(u64),
    BibDone(u64, PathBuf),
    Jump(PathBuf, usize, usize),
    Reveal(u64, Value),
}

pub fn log(m: &str) {
    eprintln!("[ltxp] {m}");
}

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version" | "-V") => return println!("ltxp {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => return println!("{USAGE}"),
        Some("uninstall") => {
            let yes = std::env::args()
                .nth(2)
                .is_some_and(|a| a == "--yes" || a == "-y");
            std::process::exit(uninstall::uninstall(yes));
        }
        _ => {}
    }
    let Some(native) = find_native() else {
        return log(&format!(
            "cannot find {NATIVE} next to the ltxp binary; set LTXP_NATIVE"
        ));
    };
    let (tx, rx) = channel();
    handle_signals(tx.clone());
    let lsp_mode = arg.is_none();
    if lsp_mode {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            while let Some(v) = lsp::read(&mut stdin) {
                if tx.send(Msg::Lsp(v)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Quit);
        });
    }
    let ttx = tx.clone();
    std::thread::spawn(move || {
        while ttx.send(Msg::Tick).is_ok() {
            std::thread::sleep(GC_INTERVAL);
        }
    });
    let mut srv = Server {
        open_browser: true,
        tx,
        lsp: lsp_mode,
        native,
        docs: HashMap::new(),
        project: None,
        next_id: 1,
        reveal_seq: 0,
    };
    if let Some(f) = arg {
        let Ok(main) = std::fs::canonicalize(&f) else {
            return log(&format!("no such file: {f}"));
        };
        srv.start_project(main);
    }
    for msg in rx {
        // A bug must not orphan the TeX processes: log it and keep serving.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| srv.handle(msg)));
        if r.is_err() {
            log("internal error while handling a message; continuing");
        }
    }
}

/// File name of the LuaTeX module built by `ltxpfork`.
const NATIVE: &str = if cfg!(target_os = "macos") {
    "libltxpfork.dylib"
} else {
    "libltxpfork.so"
};

/// `LTXP_NATIVE`, else `NATIVE` next to the binary or in `../lib/ltxp/`.
fn find_native() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LTXP_NATIVE") {
        return Some(p.into());
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(NATIVE), dir.join("../lib/ltxp").join(NATIVE)]
        .into_iter()
        .find(|p| p.exists())
}

/// Write end of the self-pipe the signal handler pokes.
static SIGNAL_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    let fd = SIGNAL_FD.load(std::sync::atomic::Ordering::Relaxed);
    unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) };
}

/// Turns SIGTERM, SIGHUP and SIGINT into `Msg::Quit` so TeX processes are always killed.
fn handle_signals(tx: Sender<Msg>) {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    for fd in fds {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    SIGNAL_FD.store(fds[1], std::sync::atomic::Ordering::Relaxed);
    for s in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        unsafe { libc::signal(s, on_signal as *const () as libc::sighandler_t) };
    }
    std::thread::spawn(move || {
        let mut b = 0u8;
        if unsafe { libc::read(fds[0], (&mut b as *mut u8).cast(), 1) } == 1 {
            let _ = tx.send(Msg::Quit);
        }
    });
}

struct Project {
    /// viewer address
    url: String,
    /// TeX inputs outside `root` for the watcher to poll
    watch_extra: Sender<PathBuf>,
    /// `.bib` files outside `root` already sent to the watcher
    watched_bibs: HashSet<PathBuf>,
    /// last `errors` message, replayed to viewers that connect later
    errors: Value,
    main_tex: PathBuf,
    root: PathBuf,
    run_dir: PathBuf,
    engine: Engine,
    clients: Vec<TcpStream>,
    done: Option<Pdf>,
    partial: Option<Pdf>,
    auto_reruns: u32,
    diagnosed: HashSet<PathBuf>,
    /// content hash of each watched file as last seen on disk
    disk_hash: HashMap<PathBuf, u64>,
    /// last seen disk text of TeX inputs, to find the first changed line
    disk_text: HashMap<PathBuf, String>,
}

impl Project {
    /// The PDF the viewer shows on top: the partial if newer than the done one.
    fn shown(&self) -> Option<&Pdf> {
        match (&self.partial, &self.done) {
            (Some(p), Some(d)) if p.generation > d.generation => Some(p),
            (p, None) => p.as_ref(),
            (_, d) => d.as_ref(),
        }
    }

    fn broadcast(&mut self, v: &Value) {
        let line = format!("data: {v}\n\n");
        self.clients
            .retain_mut(|c| c.write_all(line.as_bytes()).is_ok());
    }
}

fn pdf_msg(p: &Pdf, first_changed: u64) -> Value {
    let kind = if p.kind == Kind::Done {
        "done"
    } else {
        "partial"
    };
    let dir = p.dir.file_name().unwrap_or_default().to_string_lossy();
    json!({ "type": "pdf", "gen": p.generation, "kind": kind, "firstChanged": first_changed, "url": format!("/out/{dir}/main.pdf") })
}

struct Server {
    tx: Sender<Msg>,
    /// editor attached over stdin/stdout
    lsp: bool,
    native: PathBuf,
    /// text of documents open in the editor, by canonical path
    docs: HashMap<PathBuf, String>,
    project: Option<Project>,
    next_id: u64,
    reveal_seq: u64,
    /// ask the editor to open the viewer when a project starts; clients that open it
    /// themselves pass `initializationOptions.openBrowser = false`
    open_browser: bool,
}

impl Server {
    fn send(&mut self, v: Value) {
        if self.lsp {
            lsp::write(&mut std::io::stdout().lock(), v);
        }
    }

    /// `window/showMessage`; `kind` 1 is an error, 3 information. Also logged.
    fn show_message(&mut self, kind: u8, msg: &str) {
        log(msg);
        self.send(
            json!({ "method": "window/showMessage", "params": { "type": kind, "message": msg } }),
        );
    }

    fn request(&mut self, method: &str, params: Value) {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "id": id, "method": method, "params": params }));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Lsp(v) => self.on_lsp(v),
            Msg::Quit => self.quit(),
            Msg::Ev(line) => {
                if std::env::var_os("LTXP_DEBUG").is_some() {
                    let ms = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() % 100_000);
                    log(&format!(
                        "{ms} ev {}",
                        line.chars().take(60).collect::<String>()
                    ));
                }
                let Some(p) = &mut self.project else { return };
                let pdf = p.engine.on_event(&line);
                for f in p.engine.take_new_inputs() {
                    if !f.starts_with(&p.root) && !f.to_string_lossy().contains("/texmf") {
                        let _ = p.watch_extra.send(f.clone());
                    }
                    if let Ok(t) = std::fs::read_to_string(&f) {
                        p.disk_text.entry(f).or_insert(t);
                    }
                }
                if let Some(pdf) = pdf {
                    self.on_pdf(pdf);
                }
            }
            Msg::Sse(mut s) => {
                let Some(p) = &mut self.project else { return };
                let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
                let replay: String = [&p.done, &p.partial]
                    .into_iter()
                    .flatten()
                    .map(|pdf| format!("data: {}\n\n", pdf_msg(pdf, 1)))
                    .collect();
                let replay = if p.errors.is_null() {
                    replay
                } else {
                    format!("{replay}data: {}\n\n", p.errors)
                };
                if s.write_all(replay.as_bytes()).is_ok() {
                    p.clients.push(s);
                }
            }
            Msg::Click(v) => self.on_click(&v),
            Msg::Fs(path) => self.on_fs(path),
            Msg::Tick => self.gc_runs(),
            Msg::Watchdog(g) => {
                if let Some(p) = &mut self.project {
                    p.engine.on_watchdog(g);
                }
            }
            Msg::BibDone(g, dir) => {
                if let Some(p) = &mut self.project
                    && p.engine.generation() == g
                {
                    p.engine.cold(Some(&dir));
                }
            }
            Msg::Jump(file, line, col) => {
                if !self.lsp {
                    return println!("{}:{}:{}", file.display(), line + 1, col + 1);
                }
                let pos = json!({ "line": line, "character": col });
                self.request("window/showDocument", json!({ "uri": path2uri(&file), "takeFocus": true, "selection": { "start": pos, "end": pos } }));
            }
            Msg::Reveal(seq, v) => {
                if seq == self.reveal_seq
                    && let Some(p) = &mut self.project
                {
                    p.broadcast(&v);
                }
            }
        }
    }

    fn start_project(&mut self, main_tex: PathBuf) {
        let root = main_tex
            .parent()
            .map_or_else(|| PathBuf::from("/"), Path::to_path_buf);
        let base = if Path::new("/dev/shm").is_dir() {
            PathBuf::from("/dev/shm")
        } else {
            std::env::temp_dir()
        };
        let run_dir = base.join(format!(
            "ltxp-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_micros())
        ));
        let port = std::env::var("LTXP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => return log(&format!("cannot listen on port {port}: {e}")),
        };
        if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(&run_dir) {
            return log(&format!("cannot create {}: {e}", run_dir.display()));
        }
        let run_dir = std::fs::canonicalize(&run_dir).unwrap_or(run_dir);
        let engine = match Engine::new(
            main_tex.clone(),
            run_dir.clone(),
            self.native.clone(),
            self.tx.clone(),
        ) {
            Ok(e) => e,
            Err(e) => return log(&format!("cannot set up {}: {e}", run_dir.display())),
        };
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().map_or(0, |a| a.port())
        );
        http::serve(listener, run_dir.clone(), self.tx.clone());
        let (watch_tx, watch_rx) = channel();
        watch::spawn(root.clone(), watched, watch_rx, self.tx.clone());
        let mut p = Project {
            url: url.clone(),
            watch_extra: watch_tx,
            watched_bibs: HashSet::new(),
            errors: Value::Null,
            main_tex,
            root,
            run_dir,
            engine,
            clients: vec![],
            done: None,
            partial: None,
            auto_reruns: 0,
            diagnosed: HashSet::new(),
            disk_hash: HashMap::new(),
            disk_text: HashMap::new(),
        };
        for (f, t) in &self.docs {
            p.engine.write_shadow(f, t);
        }
        p.engine.cold(None);
        self.project = Some(p);
        log(&format!("viewer at {url}"));
        if which("lualatex").is_none() {
            self.show_message(
                1,
                "ltxp: lualatex not found in PATH; install TeX Live (MacTeX on macOS) or fix PATH",
            );
        }
        if self.lsp {
            if self.open_browser {
                self.show_message(3, &format!("ltxp preview: {url}"));
                self.request(
                    "window/showDocument",
                    json!({ "uri": url, "external": true }),
                );
            }
        } else {
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let _ = Command::new(opener)
                .arg(&url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }

    fn on_pdf(&mut self, pdf: Pdf) {
        let Some(p) = &mut self.project else { return };
        p.broadcast(&pdf_msg(&pdf, pdf.first_changed));
        if pdf.kind == Kind::Partial {
            p.partial = Some(pdf);
            return;
        }
        p.done = Some(pdf.clone());
        for bib in bib_files(&pdf.dir.join("main.aux"), &p.root) {
            if !bib.starts_with(&p.root) && p.watched_bibs.insert(bib.clone()) {
                let _ = p.watch_extra.send(bib);
            }
        }
        let errors = log_errors(&pdf.dir.join("main.log"));
        let no_pdf = !pdf.dir.join("main.pdf").is_file();
        p.errors = json!({ "type": "errors", "messages": errors, "noPdf": no_pdf });
        p.broadcast(&p.errors.clone());
        if no_pdf {
            let first = errors.first().map_or("see the log", String::as_str);
            self.show_message(1, &format!("ltxp: LaTeX produced no PDF: {first}"));
        }
        self.publish_diagnostics(&pdf.dir);
        self.rerun_if_stale(&pdf, false);
    }

    /// Sends the errors of the run in `dir` (`-file-line-error` log lines) for project files.
    fn publish_diagnostics(&mut self, dir: &Path) {
        let Some(p) = &mut self.project else { return };
        let log = std::fs::read(dir.join("main.log")).unwrap_or_default();
        let mut by_file: HashMap<PathBuf, Vec<Value>> =
            p.diagnosed.drain().map(|f| (f, vec![])).collect();
        for line in String::from_utf8_lossy(&log).lines() {
            let Some((file, l, msg)) = parse_error(line) else {
                continue;
            };
            let file = p.root.join(file).components().collect::<PathBuf>();
            if !file.is_file() || file.to_string_lossy().contains("/texmf") {
                continue;
            }
            let range = json!({ "start": { "line": l.saturating_sub(1), "character": 0 }, "end": { "line": l.saturating_sub(1), "character": 1000 } });
            by_file
                .entry(file)
                .or_default()
                .push(json!({ "range": range, "severity": 1, "source": "ltxp", "message": msg }));
        }
        p.diagnosed = by_file
            .iter()
            .filter(|(_, d)| !d.is_empty())
            .map(|(f, _)| f.clone())
            .collect();
        for (f, d) in by_file {
            self.send(json!({ "method": "textDocument/publishDiagnostics", "params": { "uri": path2uri(&f), "diagnostics": d } }));
        }
    }

    /// Cross-references and citations settle over runs: when a finished run wrote aux files
    /// different from those it started with, runs bibtex/biber if needed and compiles again.
    fn rerun_if_stale(&mut self, pdf: &Pdf, force: bool) {
        let Some(p) = &mut self.project else { return };
        if !force && (seed_sig(&pdf.dir) == pdf.seed || p.auto_reruns >= MAX_AUTO_RERUNS) {
            return;
        }
        p.auto_reruns += 1;
        let aux = std::fs::read_to_string(pdf.dir.join("main.aux")).unwrap_or_default();
        let bib = if pdf.dir.join("main.bcf").exists() {
            Some("biber")
        } else {
            aux.contains("\\bibdata").then_some("bibtex")
        };
        let Some(bib) = bib else {
            return p.engine.cold(Some(&pdf.dir));
        };
        let (tx, generation, dir, root) = (
            self.tx.clone(),
            p.engine.generation(),
            pdf.dir.clone(),
            p.root.clone(),
        );
        std::thread::spawn(move || {
            // The run dir holds files the document writes itself (`filecontents`).
            let inputs = format!("{}:{}:", dir.display(), root.display());
            // bibtex runs in the project dir like a normal build, so `\bibliography{../refs}`
            // resolves there, and writes `main.bbl` next to the run's `main.aux`.
            let mut cmd = Command::new(bib);
            if bib == "bibtex" {
                cmd.arg(dir.join("main"))
                    .current_dir(&root)
                    .env("openout_any", "a");
            } else {
                cmd.arg("main").current_dir(&dir);
            }
            let _ = cmd
                .env("BIBINPUTS", &inputs)
                .env("BSTINPUTS", &inputs)
                .output();
            let _ = tx.send(Msg::BibDone(generation, dir));
        });
    }

    /// Removes run dirs the viewer no longer needs.
    fn gc_runs(&mut self) {
        let Some(p) = &self.project else { return };
        let keep: Vec<&Path> = [&p.done, &p.partial]
            .into_iter()
            .flatten()
            .map(|pdf| pdf.dir.as_path())
            .collect();
        let live = format!("g{}", p.engine.generation());
        for e in std::fs::read_dir(&p.run_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_run = name.starts_with('g')
                && name[1..]
                    .split('p')
                    .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !is_run
                || keep.contains(&e.path().as_path())
                || name == live
                || name.starts_with(&format!("{live}p"))
            {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|d| d > GC_INTERVAL));
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    /// Viewer click: jump the editor to the exact source character.
    fn on_click(&mut self, v: &Value) {
        let Some(p) = &self.project else { return };
        let url = v["url"].as_str().unwrap_or("");
        let dir = p.run_dir.join(
            Path::new(url)
                .parent()
                .and_then(Path::file_name)
                .unwrap_or_default(),
        );
        if !dir.join("main.synctex").exists() {
            return;
        }
        let chars = |k: &str| v[k].as_str().unwrap_or("").chars().collect();
        let click = sync::Click {
            pdf: dir.join("main.pdf"),
            page: v["page"].as_u64().unwrap_or(1),
            x: v["x"].as_f64().unwrap_or(0.0),
            y: v["y"].as_f64().unwrap_or(0.0),
            before: chars("before"),
            after: chars("after"),
        };
        let (docs, tx) = (self.docs.clone(), self.tx.clone());
        std::thread::spawn(move || {
            if let Some((f, l, c)) = sync::inverse(&click, |f| read_text(&docs, f)) {
                let _ = tx.send(Msg::Jump(f, l, c));
            }
        });
    }

    /// Editor cursor: show the exact spot in the viewer.
    fn reveal(&mut self, uri: &str, pos: &Value) {
        let Some(pdf) = self
            .project
            .as_ref()
            .and_then(Project::shown)
            .map(|pdf| pdf.dir.join("main.pdf"))
        else {
            return;
        };
        let file = uri2path(uri);
        let Some(text) = read_text(&self.docs, &file) else {
            return;
        };
        let (line, col) = (
            pos["line"].as_u64().unwrap_or(0) as usize,
            pos["character"].as_u64().unwrap_or(0) as usize,
        );
        self.reveal_seq += 1;
        let (seq, tx) = (self.reveal_seq, self.tx.clone());
        std::thread::spawn(move || {
            if let Some(mut v) = sync::forward(&pdf, &file, line, col, &text) {
                v["type"] = "reveal".into();
                let _ = tx.send(Msg::Reveal(seq, v));
            }
        });
    }

    /// An editor buffer changed from 0-based `first_line` on.
    fn on_edit(&mut self, file: &Path, first_line: usize) {
        let Some(p) = &mut self.project else {
            let main = self.docs.get(file).and_then(|t| find_main(file, t));
            return main.into_iter().for_each(|m| self.start_project(m));
        };
        let Some(text) = self.docs.get(file) else {
            return;
        };
        if file.extension().is_some_and(|e| e == "bib") {
            return;
        }
        p.engine.write_shadow(file, text);
        p.auto_reruns = 0;
        p.engine.edit(file, first_line + 1);
    }

    /// A file in the project directory was written by something other than the editor.
    fn on_fs(&mut self, path: PathBuf) {
        let Some(p) = &mut self.project else { return };
        if self.docs.contains_key(&path) {
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let mut h = DefaultHasher::new();
        bytes.hash(&mut h);
        if p.disk_hash.insert(path.clone(), h.finish()) == Some(h.finish()) {
            return;
        }
        let is_input =
            path == p.main_tex || p.engine.reads(&path) || p.disk_text.contains_key(&path);
        if !is_input && p.engine.busy() {
            // Written while TeX runs: attributed to shell-escape tools, whose output may differ
            // every run; reacting would loop.
            return;
        }
        p.auto_reruns = 0;
        if is_input {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if p.disk_text.get(&path) == Some(&text) {
                return;
            }
            let line = p
                .disk_text
                .get(&path)
                .map_or(0, |old| first_diff_line(old, &text));
            p.disk_text.insert(path.clone(), text);
            log(&format!("{} changed on disk", path.display()));
            return p.engine.edit(&path, line + 1);
        }
        log(&format!("{} changed on disk", path.display()));
        let Some(done) = p.done.clone() else {
            return p.engine.cold(None);
        };
        if path.extension().is_some_and(|e| e == "bib") {
            return self.rerun_if_stale(&done, true);
        }
        p.engine.cold(Some(&done.dir));
    }

    fn on_lsp(&mut self, msg: Value) {
        let (id, method, params) = (
            &msg["id"],
            msg["method"].as_str().unwrap_or(""),
            &msg["params"],
        );
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        match method {
            "initialize" => {
                self.open_browser = params["initializationOptions"]["openBrowser"].as_bool().unwrap_or(true);
                self.send(json!({ "id": id, "result": {
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 2, "save": true },
                    "executeCommandProvider": { "commands": ["ltxp.forwardSearch", "ltxp.previewUrl", "ltxp.logPath"] },
                },
                "serverInfo": { "name": "ltxp", "version": env!("CARGO_PKG_VERSION") },
            }}));
            }
            "textDocument/didOpen" => {
                let file = uri2path(uri);
                let text = params["textDocument"]["text"].as_str().unwrap_or("").to_string();
                let disk = std::fs::read_to_string(&file).unwrap_or_default();
                let line = first_diff_line(&disk, &text);
                let changed = disk != text;
                self.docs.insert(file.clone(), text);
                if self.project.is_none() || changed {
                    self.on_edit(&file, line);
                }
            }
            "textDocument/didChange" => {
                let file = uri2path(uri);
                let Some(text) = self.docs.get_mut(&file) else { return };
                let line = apply_changes(text, params["contentChanges"].as_array().map_or(&[][..], Vec::as_slice));
                self.on_edit(&file, line);
            }
            "textDocument/didSave" => {
                let file = uri2path(uri);
                if let Some(p) = &mut self.project
                    && let Some(done) = p.done.clone()
                    && file.extension().is_some_and(|e| e == "bib")
                {
                    p.auto_reruns = 0;
                    self.rerun_if_stale(&done, true);
                }
            }
            "textDocument/didClose" => {
                let file = uri2path(uri);
                let Some(text) = self.docs.remove(&file) else { return };
                let Some(p) = &mut self.project else { return };
                let disk = std::fs::read_to_string(&file).unwrap_or_default();
                p.engine.drop_shadow(&file);
                if disk != text {
                    p.engine.edit(&file, first_diff_line(&text, &disk) + 1);
                }
            }
            "ltxp/cursor" => self.reveal(uri, &params["position"]),
            "workspace/executeCommand" => {
                let mut result = Value::Null;
                match params["command"].as_str() {
                    Some("ltxp.forwardSearch") => {
                        let args = &params["arguments"];
                        self.reveal(args[0].as_str().unwrap_or(""), &args[1]);
                    }
                    Some("ltxp.logPath") => {
                        result = self.project.as_ref().and_then(|p| p.done.as_ref()).map_or(Value::Null, |d| {
                            // Copy, since the run dir it lives in is garbage-collected.
                            let log = copy_log(&d.dir);
                            log.map_or(Value::Null, |l| l.display().to_string().into())
                        })
                    }
                    Some("ltxp.previewUrl") => result = self.project.as_ref().map_or(Value::Null, |p| p.url.clone().into()),
                    _ => {}
                }
                self.send(json!({ "id": id, "result": result }));
            }
            "shutdown" => self.send(json!({ "id": id, "result": null })),
            "exit" => self.quit(),
            "" => {}
            _ if !id.is_null() => self.send(json!({ "id": id, "error": { "code": -32601, "message": format!("unsupported: {method}") } })),
            _ => {}
        }
    }

    /// Kills all TeX processes and removes scratch files.
    fn quit(&mut self) {
        if let Some(p) = &mut self.project {
            p.engine.close();
            let _ = std::fs::remove_dir_all(&p.run_dir);
        }
        std::process::exit(0);
    }
}

/// Files whose change on disk recompiles: watched extensions, not editor backups.
fn watched(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext_ok = path
        .extension()
        .is_some_and(|e| WATCHED_EXTS.iter().any(|w| e.eq_ignore_ascii_case(w)));
    ext_ok && !name.ends_with('~')
}

/// Path of executable `name` on PATH.
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn read_text(docs: &HashMap<PathBuf, String>, file: &Path) -> Option<String> {
    docs.get(file)
        .cloned()
        .or_else(|| std::fs::read_to_string(file).ok())
}

/// `% !TEX root = x` magic comment, else the file itself if it has `\documentclass`.
fn find_main(file: &Path, text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let Some(c) = line.trim_start().strip_prefix('%') else {
            continue;
        };
        let c = c.trim_start();
        let is_root = c
            .as_bytes()
            .get(..9)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"!tex root"));
        let Some(root) = c
            .get(9..)
            .filter(|_| is_root)
            .and_then(|r| r.trim_start().strip_prefix('='))
        else {
            continue;
        };
        let p = file.parent()?.join(root.trim());
        return Some(std::fs::canonicalize(&p).unwrap_or(p));
    }
    if has_documentclass(text) {
        return Some(file.to_path_buf());
    }
    find_includer(file)
}

fn has_documentclass(text: &str) -> bool {
    text.lines().any(|l| {
        l.split('%')
            .next()
            .unwrap_or("")
            .contains("\\documentclass")
    })
}

/// The main file of a sub-file opened alone: a `.tex` file with `\documentclass` that refers to
/// `file` by its relative path (e.g. `\input{../sections/intro}`), else one that mentions its
/// name. Searches folders up to the git root (at most `MAIN_SEARCH_UP` levels), nearest first.
fn find_includer(file: &Path) -> Option<PathBuf> {
    let stem = file.file_stem()?.to_str()?;
    let mut by_name = None;
    let mut seen = HashSet::new();
    for (up, dir) in file.ancestors().skip(1).enumerate() {
        if up > MAIN_SEARCH_UP {
            break;
        }
        for cand in tex_files(dir, &mut seen) {
            let Ok(t) = std::fs::read_to_string(&cand) else {
                continue;
            };
            if cand == file || !has_documentclass(&t) || !t.contains(stem) {
                continue;
            }
            let rel = relative(cand.parent()?, &file.with_extension(""));
            if t.contains(&rel) {
                return Some(cand);
            }
            by_name.get_or_insert(cand);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    by_name
}

/// `.tex` files under `dir` (depth ≤ 3, non-hidden) not in `seen`, which collects them.
fn tex_files(dir: &Path, seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
    let (mut out, mut stack) = (vec![], vec![(dir.to_path_buf(), 0)]);
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_name().as_encoded_bytes().starts_with(b".") || seen.len() > MAIN_SEARCH_FILES
            {
                continue;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < 3 {
                    stack.push((p, depth + 1));
                }
            } else if p.extension().is_some_and(|x| x == "tex") && seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    out
}

/// `to` relative to directory `from`, with `/` separators, e.g. `../sections/intro`.
fn relative(from: &Path, to: &Path) -> String {
    let (f, t): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let ups = std::iter::repeat_n("..".to_string(), f.len() - common);
    let downs = t[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    ups.chain(downs).collect::<Vec<_>>().join("/")
}

/// `.bib` files named by `\bibdata{a,b}` in `aux`, relative to the project dir `root`.
fn bib_files(aux: &Path, root: &Path) -> Vec<PathBuf> {
    let aux = std::fs::read_to_string(aux).unwrap_or_default();
    aux.lines()
        .filter_map(|l| l.strip_prefix("\\bibdata{")?.strip_suffix('}'))
        .flat_map(|names| names.split(','))
        .map(|n| {
            root.join(if n.ends_with(".bib") {
                n.to_string()
            } else {
                format!("{n}.bib")
            })
        })
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect()
}

/// Copies the log of the run in `dir` to `ltxp.log` in the run dir's parent, which garbage
/// collection keeps; returns that path.
fn copy_log(dir: &Path) -> Option<PathBuf> {
    let dst = dir.parent()?.join("ltxp.log");
    std::fs::copy(dir.join("main.log"), &dst).ok()?;
    Some(dst)
}

/// The first few error messages in a TeX log: `FILE:LINE: MESSAGE` or `! MESSAGE` lines.
fn log_errors(log: &Path) -> Vec<String> {
    let log = std::fs::read(log).unwrap_or_default();
    String::from_utf8_lossy(&log)
        .lines()
        .filter_map(|l| match parse_error(l) {
            Some((file, line, msg)) => Some(format!("{file}:{line}: {msg}")),
            None => l.strip_prefix("! ").map(str::to_string),
        })
        .take(5)
        .collect()
}

/// `FILE:LINE: MESSAGE` from a `-file-line-error` log line.
fn parse_error(line: &str) -> Option<(&str, usize, &str)> {
    let mut start = 0;
    while let Some(i) = line[start..].find(':').map(|i| start + i) {
        let rest = &line[i + 1..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let file = &line[..i];
        if digits > 0
            && rest[digits..].starts_with(": ")
            && file.rsplit('/').next().is_some_and(|b| b.contains('.'))
        {
            return Some((file, rest[..digits].parse().ok()?, &rest[digits + 2..]));
        }
        start = i + 1;
    }
    None
}

/// Applies LSP incremental changes; returns the first changed 0-based line.
fn apply_changes(text: &mut String, changes: &[Value]) -> usize {
    let mut first = usize::MAX;
    for c in changes {
        let new = c["text"].as_str().unwrap_or("");
        if c["range"].is_null() {
            *text = new.to_string();
            first = 0;
            continue;
        }
        let starts = sync::line_starts(text);
        let off = |k: &str| {
            let p = &c["range"][k];
            sync::pos2byte(
                text,
                &starts,
                p["line"].as_u64().unwrap_or(0) as usize,
                p["character"].as_u64().unwrap_or(0) as usize,
            )
        };
        let (a, b) = (off("start"), off("end"));
        first = first.min(c["range"]["start"]["line"].as_u64().unwrap_or(0) as usize);
        text.replace_range(a..b.max(a), new);
    }
    first
}

fn first_diff_line(a: &str, b: &str) -> usize {
    a.split('\n')
        .zip(b.split('\n'))
        .take_while(|(x, y)| x == y)
        .count()
}

fn uri2path(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri).as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = raw
            .get(i + 1..i + 3)
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (raw[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    let p = PathBuf::from(std::ffi::OsString::from(
        String::from_utf8_lossy(&out).into_owned(),
    ));
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn path2uri(p: &Path) -> String {
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}
