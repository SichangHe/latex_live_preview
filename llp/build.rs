//! Fetches the pinned pdf.js build into `OUT_DIR` for embedding, unless `LLP_PDFJS_DIR` already
//! holds `pdf.min.mjs`, `pdf.worker.min.mjs` and `pdf_viewer.css`.
use std::path::{Path, PathBuf};
use std::process::Command;

const PDFJS_VERSION: &str = "4.10.38";
const FILES: [(&str, &str); 3] = [
    ("package/build/pdf.min.mjs", "pdf.min.mjs"),
    ("package/build/pdf.worker.min.mjs", "pdf.worker.min.mjs"),
    ("package/web/pdf_viewer.css", "pdf_viewer.css"),
];

fn main() {
    println!("cargo:rerun-if-env-changed=LLP_PDFJS_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Ok(dir) = std::env::var("LLP_PDFJS_DIR") {
        for (_, name) in FILES {
            std::fs::copy(Path::new(&dir).join(name), out.join(name))
                .expect("LLP_PDFJS_DIR lacks a pdf.js file");
        }
        return;
    }
    if FILES.iter().all(|(_, name)| out.join(name).exists()) {
        return;
    }
    let tgz = out.join("pdfjs.tgz");
    let url = format!("https://registry.npmjs.org/pdfjs-dist/-/pdfjs-dist-{PDFJS_VERSION}.tgz");
    let ok = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(&tgz)
        .arg(&url)
        .status()
        .is_ok_and(|s| s.success());
    assert!(
        ok,
        "downloading {url} failed; set LLP_PDFJS_DIR to a directory with the pdf.js files"
    );
    let ok = Command::new("tar")
        .arg("-xzf")
        .arg(&tgz)
        .arg("-C")
        .arg(&out)
        .args(FILES.map(|(p, _)| p))
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "extracting pdf.js failed");
    for (p, name) in FILES {
        std::fs::rename(out.join(p), out.join(name)).expect("extracted pdf.js file exists");
    }
}
