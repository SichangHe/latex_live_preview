//! Viewer HTTP server on 127.0.0.1: embedded assets, run-dir PDFs with range requests, the SSE
//! stream and click posts. Each connection gets a thread; state changes go to the actor.
use crate::Msg;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

const ASSETS: [(&str, &str, &[u8]); 6] = [
    ("/", "text/html", include_bytes!("../../viewer/index.html")),
    (
        "/viewer.js",
        "text/javascript",
        include_bytes!("../../viewer/viewer.js"),
    ),
    (
        "/match.js",
        "text/javascript",
        include_bytes!("../../viewer/match.js"),
    ),
    (
        "/pdf.mjs",
        "text/javascript",
        include_bytes!(concat!(env!("OUT_DIR"), "/pdf.min.mjs")),
    ),
    (
        "/pdf.worker.mjs",
        "text/javascript",
        include_bytes!(concat!(env!("OUT_DIR"), "/pdf.worker.min.mjs")),
    ),
    (
        "/pdf_viewer.css",
        "text/css",
        include_bytes!(concat!(env!("OUT_DIR"), "/pdf_viewer.css")),
    ),
];

pub fn serve(listener: TcpListener, run_dir: PathBuf, tx: Sender<Msg>) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (run_dir, tx) = (run_dir.clone(), tx.clone());
            std::thread::spawn(move || {
                let _ = handle(stream, port, &run_dir, &tx);
            });
        }
    });
}

struct Req {
    method: String,
    path: String,
    host: String,
    range: Option<(u64, Option<u64>)>,
    body: Vec<u8>,
}

fn parse(r: &mut BufReader<TcpStream>) -> std::io::Result<Req> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let (method, path) = (
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    );
    let (mut host, mut range, mut len) = (String::new(), None, 0);
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.to_ascii_lowercase().as_str() {
            "host" => host = v.to_string(),
            "content-length" => len = v.parse().unwrap_or(0),
            "range" => {
                range = v
                    .strip_prefix("bytes=")
                    .and_then(|s| s.split_once('-'))
                    .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok())));
            }
            _ => {}
        }
    }
    let mut body = vec![0; len.min(1 << 20)];
    r.read_exact(&mut body)?;
    Ok(Req {
        method,
        path,
        host,
        range,
        body,
    })
}

fn respond(
    s: &mut TcpStream,
    status: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nconnection: close\r\ncache-control: no-store\r\n"
    )?;
    for (k, v) in headers {
        write!(s, "{k}: {v}\r\n")?;
    }
    write!(s, "content-length: {}\r\n\r\n", body.len())?;
    s.write_all(body)
}

fn handle(
    stream: TcpStream,
    port: u16,
    run_dir: &std::path::Path,
    tx: &Sender<Msg>,
) -> std::io::Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let req = parse(&mut r)?;
    let mut s = stream;
    if req.host != format!("127.0.0.1:{port}") && req.host != format!("localhost:{port}") {
        return respond(&mut s, "403 Forbidden", &[], b"");
    }
    let path = req.path.split('?').next().unwrap_or("");
    if path == "/events" {
        write!(
            s,
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-store\r\n\r\n"
        )?;
        let _ = tx.send(Msg::Sse(s));
        return Ok(());
    }
    if path == "/click" && req.method == "POST" {
        if let Ok(v) = serde_json::from_slice(&req.body) {
            let _ = tx.send(Msg::Click(v));
        }
        return respond(&mut s, "204 No Content", &[], b"");
    }
    if let Some((_, ty, body)) = ASSETS.iter().find(|(p, _, _)| *p == path) {
        return respond(&mut s, "200 OK", &[("content-type", ty.to_string())], body);
    }
    let dir = path
        .strip_prefix("/out/")
        .and_then(|p| p.strip_suffix("/main.pdf"));
    let valid = dir.is_some_and(|d| {
        d.starts_with('g')
            && d[1..]
                .split('p')
                .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    });
    let Some(dir) = dir.filter(|_| valid) else {
        return respond(&mut s, "404 Not Found", &[], b"");
    };
    let Ok(mut f) = File::open(run_dir.join(dir).join("main.pdf")) else {
        return respond(&mut s, "404 Not Found", &[], b"");
    };
    let size = f.metadata()?.len();
    let pdf = [
        ("content-type", "application/pdf".to_string()),
        ("accept-ranges", "bytes".to_string()),
    ];
    let Some((start, end)) = req.range else {
        let mut body = Vec::with_capacity(size as usize);
        f.read_to_end(&mut body)?;
        return respond(&mut s, "200 OK", &pdf, &body);
    };
    let end = end.unwrap_or(u64::MAX).min(size.saturating_sub(1));
    if start > end {
        return respond(&mut s, "416 Range Not Satisfiable", &[], b"");
    }
    f.seek(SeekFrom::Start(start))?;
    let mut body = vec![0; (end - start + 1) as usize];
    f.read_exact(&mut body)?;
    let mut headers = pdf.to_vec();
    headers.push(("content-range", format!("bytes {start}-{end}/{size}")));
    respond(&mut s, "206 Partial Content", &headers, &body)
}
