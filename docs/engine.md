# Engine
(authored by agents unless marked 🧑)

Terms: see `ubiquitous-language.md`.

goal: edit → updated page in a browser in ~0.1–0.2 s, independent of document length
- full compiles take seconds for long docs, so only pages from the edit on are recompiled

why LuaLaTeX
- Lua callbacks let us own file reading and hook page shipout
- a C module loaded via `package.loadlib` (needs `--shell-escape`) gives `fork`
- Tectonic/XeTeX/pdfTeX have no such hooks

pieces
- `native/llpfork.c`: fork with recorded fd offsets, detach, FIFO, `_exit`
    - Lua headers are absent, so the few Lua API prototypes are declared by hand
- `engine/driver.lua`: loaded by `\directlua` before `\input{main}`
- `server/engine.js`: actor owning gens and snapshots

driver
- `open_read_file`: reads whole files into memory (shadow copy first), serves lines
    - records max-read per file
    - resumed clones reload the content of still-open files and continue at the same line
- `finish_pdfpage`: after each page
    - fork a snapshot, which parks
    - fork a finisher if ≥ `FINISH_GAP_S` since the last one
- finisher ends at the next `buildpage_filter` via `tex.finish`
    - calling it inside `finish_pdfpage` drops the page just shipped
- `wrapup_run`: outputs are closed; emit `done`/`partial` then `_exit`, skipping ~30 ms teardown
- clones stop Lua GC: a collection touches the whole inherited heap and forces copy-on-write of it
- events go to FIFO `ev` as `GEN KIND ...` lines

output isolation
- TeX runs with `-output-directory=/proc/self/fd/200`
    - each process's own fd 200 decides where its outputs go
    - `bash -c 'exec 200<DIR; exec lualatex ...'` sets it for cold gens
- detach: dup a new dir onto fd 200; copy every writable file's prefix (up to the offset recorded at fork) into it and dup it over the old fd
    - names and offsets are recorded at fork: the parent may later rename (`main.synctex(busy)`) or delete files
    - works because TeX's outputs are append-only
- cwd stays the project dir, like a normal build, so shell-escape artifacts (e.g. `epstopdf`) are shared

server-side engine
- edit(file, line): resume the latest non-stale snapshot; drop stale ones after sending `go`
    - none: cold gen
- cold gens keep existing snapshots until their own replace them page by page
- only events of the current gen count; older gens' snapshots are told to quit
- snapshots ~5 MB each: above `LLP_MAX_SNAPS` (400) keep only pages divisible by a doubling stride

main (`server/main.js`)
- auto rerun after `done` when aux files changed vs seed, at most `MAX_AUTO_RERUNS` in a row
    - runs `bibtex`/`biber` first when needed; saving a `.bib` forces this
- diagnostics: `-file-line-error` lines of the done log
- GC of old run dirs every 10 s

measured (generated docs, this machine)
- 256 pages: cold 5.5 s; edit at page 170 → partial PDF ~0.14 s; finisher's PDF close ~60 ms of that
- keystroke → page on screen ~0.22 s through Neovim and Chromium
