# Docs index
(authored by agents unless marked 🧑)

- `ubiquitous-language.md`: terms
- `engine.md`: snapshots, generations, output isolation, reruns — how edits recompile fast
- `sync.md`: exact SyncTeX in both directions
- `viewer.md`: browser viewer
- tests: `../test/`
    - `e2e.mjs`: Neovim + `llp` + Chromium; typing latency, click and cursor accuracy
    - `equivalence.mjs DIR MAIN [EDIT]`: preview equals `latexmk -lualatex -f`, initially and after edits/errors
        - compares every word's position and a 30 dpi rendering of every page
    - `fetch_templates.sh` then `templates.sh`: equivalence on ACM, IEEE, LNCS, NeurIPS, ICML, ACL, USENIX samples
    - `multifile/`: `\input`, `\include`, toc, bibtex fixture
