# ltxp: instant LaTeX preview
(authored by agents unless marked 🧑)

Type in your editor and the PDF in your browser updates about 0.2 s later, even in long documents. Click the preview and the editor cursor lands on the exact character you clicked. Move the cursor and the preview marks the exact spot.

ltxp is a language server written in Rust. Each edit recompiles only from the edited page onward, by resuming a saved copy of the LuaLaTeX process. See `docs/index.md`.

## Install

You need Linux and TeX Live with `lualatex` and `synctex`.

```sh
curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
```

This installs `ltxp` into `~/.local/bin` and the Neovim plugin into Neovim's package path. Opening a `.tex` file then starts the preview, with no configuration needed.

To build from source, which needs Rust and `curl`:

```sh
cargo build --release   # target/release/ltxp and target/release/libltxpfork.so
```

## Use

- Neovim: open a `.tex` file that has `\documentclass`, or a `% !TEX root = main.tex` comment. The viewer opens in your browser, and its URL is also shown as a message.
  - Options: `require("ltxp").setup({ cmd = { "/path/to/ltxp" }, cursor_delay_ms = 100 })`. Set `vim.g.ltxp_disable = true` to turn the plugin off.
- Other editors: run `ltxp` as a language server for LaTeX. Without the Neovim plugin, forward search is the command `ltxp.forwardSearch` with arguments `[uri, position]`.
- No editor: `ltxp main.tex` previews the file and recompiles whenever files change on disk, like `latexmk -pvc`. Clicks print `file:line:column`.

Changes on disk are picked up in every mode, including `\input` files, images, `.bib` files and packages in the project directory. In the editor, open files update on every keystroke, without saving.

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
