# ltxp: instant LaTeX preview
(authored by agents unless marked 🧑)

Type in your editor and the PDF in your browser updates about 0.2 s later, even in long documents. Click the preview and the editor cursor lands on the exact character you clicked. Move the cursor and the preview marks the exact spot.

ltxp is a language server written in Rust. Each edit recompiles only from the edited page onward, by resuming a saved copy of the LuaLaTeX process. See `docs/index.md`.

## Install

You need Linux or macOS, and TeX Live (MacTeX on macOS) with `lualatex` and `synctex`.

```sh
curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
```

This installs `ltxp` into `~/.local/bin`. In a terminal, the installer then asks which editors to set up: Neovim, Emacs (29+), VS Code, or VSCodium. To choose directly, replace `sh` with `sh -s -- --editor nvim` (or `emacs`, `code`, `codium`); repeat `--editor` for several, or pass `--no-editor`. Add `--dir DIR` to install elsewhere, or `--release TAG` for another release.

To remove everything the installer set up (the binary, plugin files, the lazy.nvim spec, the Emacs loader line and the VS Code extension), run `ltxp uninstall`. It lists what it will remove and asks first; `--yes` skips the question.

To build from source, which needs Rust and `curl`:

```sh
cargo build --release   # target/release/ltxp and the LuaTeX module libltxpfork.so/.dylib
```

## Use

Nothing starts when you open a file. In a `.tex` file, start the preview explicitly; it opens in your browser. ltxp finds the main file from `\documentclass`, a `% !TEX root = ../paper/main.tex` comment (path relative to the open file), or else by searching up to the git root for a file with `\documentclass` that `\input`s the open file. Relative paths (`\input{../sections/x}`, images, `\bibliography{../refs}`) resolve from the main file's folder, as with latexmk. If LaTeX fails, the error shows at the top of the preview and in the editor.

- Neovim: `:LtxpPreview` starts it (again: reopens the browser), `:LtxpStop` stops it, `:LtxpLog` opens the latest LaTeX log, `:LtxpSync [on|off]` toggles the preview following the cursor. The installer registers the plugin with lazy.nvim, loaded only when you run one of these. Options: `opts = { cmd = { "/path/to/ltxp" }, cursor_delay_ms = 100 }`.
- Emacs (29+, built-in Eglot): `M-x ltxp-preview`, `M-x ltxp-stop`, `M-x ltxp-log`, `M-x ltxp-toggle-sync`. The installer adds a loader to your init file; by hand, `(load "/path/to/ltxp.el")` then `(ltxp-setup)`.
- VS Code / VSCodium: the commands "ltxp: Open preview", "ltxp: Stop preview", "ltxp: Open LaTeX log" and "ltxp: Toggle preview following the cursor". Settings are `ltxp.path` and `ltxp.cursorDelayMs`.
- Other editors: run `ltxp` as a language server for LaTeX. Forward search is the command `ltxp.forwardSearch` with arguments `[uri, position]`, or the notification `ltxp/cursor`.
- No editor: `ltxp main.tex` previews the file and recompiles whenever files change on disk, like `latexmk -pvc`. Clicks print `file:line:column`.

Changes on disk are picked up in every mode, including `\input` files, images, `.bib` files and packages in the project folder. In the editor, open files update on every keystroke, without saving.

Viewer keys: `+`, `-` and `0` zoom.

## Environment

- `LTXP_PORT`: viewer port (default: random)
- `LTXP_MAX_SNAPS`: saved process copies to keep (default 400, about 5 MB each)

## Limits

- LuaLaTeX only. Templates that use pdfTeX-only primitives show the same errors as running `lualatex` on them yourself.
- `--shell-escape` is always on, so only open documents you trust.
- Clicks inside math land at the start of the source line.

## License

[Universal Permissive License 1.0](LICENSE)
