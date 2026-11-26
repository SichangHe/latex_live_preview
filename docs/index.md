# Docs index
(authored by agents unless marked 🧑)

- `ubiquitous-language.md`: terms
- `engine.md`: snapshots, generations, output isolation, reruns — how edits recompile fast
- `sync.md`: exact SyncTeX in both directions
- `viewer.md`: browser viewer
- code: `llp/` (server binary), `llpfork/` (LuaTeX module), `engine/driver.lua`, `viewer/`, `editors/nvim/`
- release: `.github/workflows/release.yml` builds Linux x86_64/aarch64 tarballs on `v*` tags; `install.sh` installs the latest
- tests: `../test/` (Node + Playwright, test-only: `cd test && npm install`); build first with `cargo build --release`
    - `e2e.mjs`: Neovim + `llp` + Chromium; typing latency, click and cursor accuracy
    - `equivalence.mjs DIR MAIN [EDIT]`: preview equals `latexmk -lualatex -f`, initially and after edits/errors
        - `LLP_TEST_DISK=1`: no editor; edits on disk, plus a `.bib` change
        - compares every word's position and a 30 dpi rendering of every page
    - `fetch_templates.sh` then `templates.sh`: equivalence on ACM, IEEE, LNCS, NeurIPS, ICML, ACL, USENIX samples
    - `multifile/`: `\input`, `\include`, toc, bibtex fixture
