# Docs index
(authored by agents unless marked 🧑)

- `ubiquitous-language.md`: terms
- `engine.md`: snapshots, generations, output isolation, reruns — how edits recompile fast
- `sync.md`: exact SyncTeX in both directions
- `viewer.md`: browser viewer
- code: `ltxp/` (server binary), `ltxpfork/` (LuaTeX module), `engine/driver.lua`, `viewer/`, `editors/{nvim,vscode,emacs}/`
- uninstall (`ltxp/src/uninstall.rs`): `install.sh` records each file it installs, each line it adds and each editor extension in `~/.local/share/ltxp/manifest`; `ltxp uninstall` undoes them. Installs from before the manifest: default locations plus a scan for generated lazy.nvim specs, the Emacs loader line and the VS Code extension
- release: `.github/workflows/release.yml` builds Linux and macOS (x86_64, aarch64) tarballs and `ltxp.vsix` on `v*` tags; `install.sh` installs the latest and sets up chosen editors
- CI (`ci.yml`, macOS): equivalence in editor and disk modes; installs the latest release and compiles
- tests: `../test/` (Node + Playwright, test-only: `cd test && npm install`); build first with `cargo build --release`
    - `e2e.mjs`: Neovim + `ltxp` + Chromium; typing latency, click and cursor accuracy
    - `equivalence.mjs DIR MAIN [EDIT]`: preview equals `latexmk -lualatex -f`, initially and after edits/errors
        - `LTXP_TEST_DISK=1`: no editor; edits on disk, plus a `.bib` change
        - compares every word's position and a 30 dpi rendering of every page
    - `fetch_templates.sh` then `templates.sh`: equivalence on ACM, IEEE, LNCS, NeurIPS, ICML, ACL, USENIX samples
    - `multifile/`: `\input`, `\include`, toc, bibtex fixture
    - `vscode/run.cjs` (under `xvfb-run -a`), `emacs/run.mjs` (docker `silex/emacs:30`): typing, cursor → preview, click → exact cursor in real VS Code and Emacs
