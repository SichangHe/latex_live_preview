# llp: instant LaTeX preview
(authored by agents unless marked 🧑)

Type in your editor and see the PDF update in a browser in about 0.2 s, even in long documents. Clicking the preview puts the editor cursor on the exact character, and moving the cursor marks the exact spot in the preview.

It is a language server. Each edit only recompiles from the page you edited onward, by resuming a saved copy of the LuaLaTeX process. See `docs/index.md`.

## Install

Needs Linux, TeX Live with `lualatex` and `synctex`, Node ≥ 20, and gcc.

```sh
npm install   # also builds native/llpfork.so
```

## Neovim

```lua
vim.opt.rtp:prepend("/path/to/latex_live_preview/editors/nvim")
require("llp").setup({ cmd = { "/path/to/latex_live_preview/server/main.js" } })
```

Opening a `.tex` file that has `\documentclass`, or a `% !TEX root = main.tex` comment, starts a compile and opens the viewer in your browser (`vim.ui.open`). The URL is also shown as a message.

Other LSP clients work as well. Without the plugin's `llp/cursor` notification, forward search is available through the command `llp.forwardSearch` with arguments `[uri, position]`.

## Viewer

- click: jump the editor to that character
- `+` / `-` / `0`: zoom

## Environment

- `LLP_PORT`: viewer port (default: random)
- `LLP_MAX_SNAPS`: saved process copies to keep (default 400, about 5 MB each)

## Limits

- LuaLaTeX only. Templates that use pdfTeX-only primitives get the same errors as running `lualatex` on them yourself.
- `--shell-escape` is always on, because the engine loads its C module through it.
