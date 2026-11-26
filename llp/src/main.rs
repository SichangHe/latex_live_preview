// 🧑 "Layout using. Latex engine, but display using a web browser. Doing some sort of. Language server or editor plugin. Ideally, mainly language server."
//! `llp`: instant LaTeX preview language server with a browser viewer; see `docs/index.md`.
//! One actor thread (`Server`) owns all state; stdin, the engine's event FIFO, HTTP
//! connections, inotify, timers and SyncTeX workers send it `Msg`s.
mod engine;
mod http;
mod lsp;
mod sync;
mod watch;

use engine::{Engine, Kind, Pdf, seed_sig};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, SystemTime};

const MAX_AUTO_RERUNS: u32 = 3;
const GC_INTERVAL: Duration = Duration::from_secs(10);
/// Extensions of files whose change on disk recompiles.
const WATCHED_EXTS: [&str; 20] = [
    "tex", "sty", "cls", "clo", "cfg", "def", "fd", "lua", "bib", "bst", "bbx", "cbx", "png",
    "jpg", "jpeg", "pdf", "eps", "svg", "csv", "tikz",
];
const USAGE: &str = "llp: instant LaTeX preview in the browser

usage:
  llp            run as a language server on stdin/stdout (for editors)
  llp FILE.tex   preview FILE.tex without an editor, recompiling on file changes
  llp --version

env: LLP_PORT (viewer port, default random), LLP_MAX_SNAPS (default 400)";

pub enum Msg {
    Lsp(Value),
    Quit,
    Ev(String),
    Sse(TcpStream),
    Click(Value),
    Fs(i32, OsString),
    Tick,
    Watchdog(u64),
    BibDone(u64, PathBuf),
    Jump(PathBuf, usize, usize),
    Reveal(u64, Value),
}

pub fn log(m: &str) {
    eprintln!("[llp] {m}");
}

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version" | "-V") => return println!("llp {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => return println!("{USAGE}"),
        _ => {}
    }
    let Some(native) = find_native() else {
        return log("cannot find libllpfork.so next to the llp binary; set LLP_NATIVE");
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
        srv.handle(msg);
    }
}

/// `LLP_NATIVE`, else `libllpfork.so` next to the binary or in `../lib/llp/`.
fn find_native() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LLP_NATIVE") {
        return Some(p.into());
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("libllpfork.so"),
        dir.join("../lib/llp/libllpfork.so"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// Turns SIGTERM, SIGHUP and SIGINT into `Msg::Quit` so TeX processes are always killed. Must
/// run before other threads start so they inherit the blocked mask.
fn handle_signals(tx: Sender<Msg>) {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        std::thread::spawn(move || {
            let mut sig = 0;
            libc::sigwait(&set, &mut sig);
            let _ = tx.send(Msg::Quit);
        });
    }
}

struct Project {
    main_tex: PathBuf,
    root: PathBuf,
    run_dir: PathBuf,
    engine: Engine,
    clients: Vec<TcpStream>,
    done: Option<Pdf>,
    partial: Option<Pdf>,
    auto_reruns: u32,
    diagnosed: HashSet<PathBuf>,
    watcher: Option<watch::Watcher>,
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
}

impl Server {
    fn send(&mut self, v: Value) {
        if self.lsp {
            lsp::write(&mut std::io::stdout().lock(), v);
        }
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
                if let Some(pdf) = self.project.as_mut().and_then(|p| p.engine.on_event(&line)) {
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
                if s.write_all(replay.as_bytes()).is_ok() {
                    p.clients.push(s);
                }
            }
            Msg::Click(v) => self.on_click(&v),
            Msg::Fs(wd, name) => self.on_fs(wd, &name),
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
            "llp-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_micros())
        ));
        let engine = match Engine::new(
            main_tex.clone(),
            run_dir.clone(),
            self.native.clone(),
            self.tx.clone(),
        ) {
            Ok(e) => e,
            Err(e) => return log(&format!("cannot set up {}: {e}", run_dir.display())),
        };
        let port = std::env::var("LLP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => return log(&format!("cannot listen on port {port}: {e}")),
        };
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().map_or(0, |a| a.port())
        );
        http::serve(listener, run_dir.clone(), self.tx.clone());
        let watcher = watch::Watcher::new(&root, self.tx.clone());
        let mut p = Project {
            main_tex,
            root,
            run_dir,
            engine,
            clients: vec![],
            done: None,
            partial: None,
            auto_reruns: 0,
            diagnosed: HashSet::new(),
            watcher,
            disk_hash: HashMap::new(),
            disk_text: HashMap::new(),
        };
        for (f, t) in &self.docs {
            p.engine.write_shadow(f, t);
        }
        p.engine.cold(None);
        self.project = Some(p);
        log(&format!("viewer at {url}"));
        if self.lsp {
            self.send(json!({ "method": "window/showMessage", "params": { "type": 3, "message": format!("LaTeX preview: {url}") } }));
            self.request(
                "window/showDocument",
                json!({ "uri": url, "external": true }),
            );
        } else {
            let _ = Command::new("xdg-open")
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
            if !file.starts_with(&p.root) {
                continue;
            }
            let range = json!({ "start": { "line": l - 1, "character": 0 }, "end": { "line": l - 1, "character": 1000 } });
            by_file
                .entry(file)
                .or_default()
                .push(json!({ "range": range, "severity": 1, "source": "llp", "message": msg }));
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
            let inputs = format!("{}:", root.display());
            let _ = Command::new(bib)
                .arg("main")
                .current_dir(&dir)
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
        if file == p.main_tex || p.engine.reads(file) {
            p.auto_reruns = 0;
            p.engine.edit(file, first_line + 1);
        }
    }

    /// A file in the project directory was written by something other than the editor.
    fn on_fs(&mut self, wd: i32, name: &std::ffi::OsStr) {
        let Some(p) = &mut self.project else { return };
        let Some(path) = p.watcher.as_ref().and_then(|w| w.path(wd, name)) else {
            return;
        };
        let n = name.to_string_lossy();
        let ext_ok = path
            .extension()
            .is_some_and(|e| WATCHED_EXTS.iter().any(|w| e.eq_ignore_ascii_case(w)));
        if n.starts_with('.') || n.ends_with('~') || !ext_ok || self.docs.contains_key(&path) {
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
        p.auto_reruns = 0;
        if path == p.main_tex || p.engine.reads(&path) {
            let text = String::from_utf8_lossy(&bytes).into_owned();
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
            "initialize" => self.send(json!({ "id": id, "result": {
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 2, "save": true },
                    "executeCommandProvider": { "commands": ["llp.forwardSearch"] },
                },
                "serverInfo": { "name": "llp", "version": env!("CARGO_PKG_VERSION") },
            }})),
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
                if disk != text && (file == p.main_tex || p.engine.reads(&file)) {
                    p.engine.edit(&file, first_diff_line(&text, &disk) + 1);
                }
            }
            "llp/cursor" => self.reveal(uri, &params["position"]),
            "workspace/executeCommand" => {
                if params["command"] == "llp.forwardSearch" {
                    let args = &params["arguments"];
                    self.reveal(args[0].as_str().unwrap_or(""), &args[1]);
                }
                self.send(json!({ "id": id, "result": null }));
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
        if c.len() > 9 && c[..9].eq_ignore_ascii_case("!tex root") {
            let root = c[9..].trim_start().strip_prefix('=')?.trim();
            let p = file.parent()?.join(root);
            return Some(std::fs::canonicalize(&p).unwrap_or(p));
        }
    }
    text.lines()
        .any(|l| {
            l.split('%')
                .next()
                .unwrap_or("")
                .contains("\\documentclass")
        })
        .then(|| file.to_path_buf())
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
