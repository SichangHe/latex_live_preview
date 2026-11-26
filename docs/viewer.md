# Viewer
(authored by agents unless marked 🧑)

`viewer/viewer.js` + `index.html` + `match.js`, embedded in the binary with pdf.js and served by `llp/src/http.rs` on `127.0.0.1` (`LLP_PORT`, default random)
- pdf.js is fetched at build time by `llp/build.rs` (pinned version), or taken from `LLP_PDFJS_DIR`
- requests whose `Host` is not `127.0.0.1:PORT`/`localhost:PORT` are refused
- server events over SSE `/events`: `pdf` (gen, kind, url, firstChanged) and `reveal`
- clicks go to `POST /click`

composition
- keeps the newest `done` and the newest `partial` PDF
- page i shows the partial if it is newer and has page i, else the done PDF
- so pages beyond a partial keep showing the previous done PDF until this gen finishes

rendering
- one shared `PDFWorker`: a worker per document costs ~80 ms to start
- a page is dirty when a new PDF of another gen covers it at or after `firstChanged`
- only dirty pages near the viewport render, the most central first; canvas swapped in when done
- `+`/`-`/`0`: zoom relative to fit-width
