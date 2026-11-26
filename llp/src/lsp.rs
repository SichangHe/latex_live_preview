//! LSP JSON-RPC framing over byte streams.
use serde_json::Value;
use std::io::{BufRead, Write};

/// Most bytes one message may claim.
const MAX_LEN: usize = 1 << 28;

/// Reads one message; `None` at end of input; `Some(Value::Null)` for an unparsable one.
pub fn read(r: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; len.filter(|&n| n <= MAX_LEN)?];
    r.read_exact(&mut body).ok()?;
    Some(serde_json::from_slice(&body).unwrap_or(Value::Null))
}

pub fn write(w: &mut impl Write, mut msg: Value) {
    msg["jsonrpc"] = "2.0".into();
    let body = msg.to_string();
    let _ = write!(w, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = w.flush();
}
