# Write LaTeX locally: instant preview and real-time collaboration
(authored by agents unless marked 🧑)

This guide sets up a local replacement for an online LaTeX editor:
- **ltxp**: the PDF updates in your browser as you type, about 0.2–0.5 s later, even in long papers. Click the PDF to jump to that spot in your editor.
- **TeamTypeV**: co-edit the same files in real time, each person in their own editor, with Git.

Your files stay on your machine, in your editor (Neovim, VS Code, VSCodium or Emacs), under your own Git.

## For AI agents

If a user pointed you here to set this up for them:
- Do the steps in order. After each step, run its **Check** and stop to report if it fails.
- Ask the user before installing system packages (step 1), and before changing their editor configuration (the `--editor` options in steps 2 and 5).
- Ask which editor they use, and whether they need collaboration (step 5) at all.
- Supported: Linux (x86_64, ARM64) and macOS (Apple Silicon, Intel). Windows is not supported; use WSL.
- Everything installs under `~/.local`; `ltxp uninstall --yes` removes ltxp again.
- On a machine without a browser, check the preview with `ltxp main.tex` and `curl -sN http://127.0.0.1:PORT/events`, using the port in the `viewer at` line it prints (or set `LTXP_PORT`). A line containing `"kind":"done"` means a PDF was made.

## 1. Install TeX Live

ltxp compiles with LuaLaTeX from a local TeX Live.

- **macOS**: install [MacTeX](https://www.tug.org/mactex/) (full, ~6 GB), or the smaller BasicTeX:
    ```sh
    brew install --cask basictex
    sudo tlmgr update --self && sudo tlmgr install latexmk
    ```
    Open a new terminal afterwards so `/Library/TeX/texbin` is on your PATH. BasicTeX lacks many packages; install missing ones with `sudo tlmgr install NAME` when LaTeX reports them.
- **Debian/Ubuntu**: `sudo apt install texlive-full` (everything, ~5 GB), or `sudo apt install texlive-latex-extra texlive-luatex texlive-bibtex-extra biber latexmk` for a smaller set.
- **Other Linux**: your distribution's TeX Live packages, or the [TeX Live installer](https://www.tug.org/texlive/quickinstall.html).

**Check**: `lualatex --version`, `synctex help` and `latexmk --version` each print something.

## 2. Install ltxp

```sh
curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
```

In a terminal, it asks which editors to set up. To choose without the question, replace the final `sh` with `sh -s -- --editor nvim` (or `--editor code`, `--editor codium`, `--editor emacs`; repeat `--editor` for several). For no editor setup, use `sh -s -- --no-editor`. It prints what it set up and what to do next.

If it says `~/.local/bin is not in your PATH`, run this line now, and add it to your shell's startup file (`~/.zshrc` or `~/.bashrc`) for future terminals:
```sh
export PATH="$HOME/.local/bin:$PATH"
```

- **Neovim**: with lazy.nvim (e.g. LazyVim), the installer adds `lua/plugins/ltxp.lua` to your config. Without it, the installer prints the lines to add.
- **VS Code / VSCodium**: the installer needs the `code`/`codium` command (in VS Code: Command Palette → "Shell Command: Install 'code' command in PATH").
- **Emacs 29+**: the installer adds one `(load ...)` line to your init file.

**Check**: `ltxp --version` prints a version.

## 3. Preview a paper

Open any `.tex` file of your project in your editor, then start the preview. Nothing runs until you do:

- **Neovim**: `:LtxpPreview` starts it; `:LtxpStop`, `:LtxpLog` (LaTeX log), `:LtxpSync` (toggle following the cursor)
- **VS Code / VSCodium**: command "ltxp: Open preview"; also "Stop preview", "Open LaTeX log", "Toggle preview following the cursor"
- **Emacs**: `M-x ltxp-preview`; `M-x ltxp-stop`, `M-x ltxp-log`, `M-x ltxp-toggle-sync`
- **No editor**: `ltxp main.tex` runs in the foreground until Ctrl-C. It opens your browser and prints the preview URL (`viewer at http://...`). It recompiles whenever you save a file in the project.

Your browser opens the PDF. Then:
- Type: the PDF updates without saving (in no-editor mode, when you save).
- Click a word in the PDF: your cursor jumps to that exact character in the source.
- Move your cursor: the PDF marks the same spot.
- LaTeX errors show at the top of the PDF and in your editor.

ltxp finds the main file when you open a sub-file (such as `sections/intro.tex`). If it picks the wrong one, put this at the top of the sub-file, with the path relative to that file:
```tex
% !TEX root = ../main.tex
```

The preview port is random unless you set `LTXP_PORT`. Over SSH: start your editor (or `ltxp main.tex`) with `LTXP_PORT=8765`, forward the port with `ssh -L 8765:127.0.0.1:8765 HOST`, and open `http://127.0.0.1:8765/` on your own machine.

**Check**: the browser shows your PDF, and a typed word appears in it within a second.

## 4. Final PDFs and moving from Overleaf

ltxp is for previewing as you write; it compiles in a temporary folder and does not write `main.pdf` into your project. For the PDF you submit, build with latexmk in the project folder:
```sh
latexmk -lualatex main.tex   # same engine as the preview
latexmk -pdf main.tex        # pdfLaTeX, if your venue requires it
```
LuaLaTeX and pdfLaTeX can break lines slightly differently, so check the final PDF if page limits are tight. `latexmk -C` removes build files.

Moving a project from Overleaf:
1. In Overleaf: Menu → Download → Source. Unzip it into a folder.
2. Optional: `git init && git add -A && git commit -m "import from Overleaf"`.
3. Open the main `.tex` file and start the preview (step 3).

Templates written for pdfLaTeX only (rare; e.g. using `\pdfpagewidth` directly) show the same errors in ltxp as in a manual `lualatex` run.

## 5. Collaborate in real time with TeamTypeV

[TeamTypeV](https://github.com/SichangHe/TeamTypeV) shares a folder between people. Each person edits in their own editor, and everyone sees the typing live. Each person can also run their own ltxp preview.

1. Every participant installs it and picks their editors:
    ```sh
    curl -fsSL https://raw.githubusercontent.com/SichangHe/TeamTypeV/live-multi-editor/install.sh | sh
    ```
2. The owner of the project runs this in the project folder, and keeps it running:
    ```sh
    teamtypev share
    ```
    It prints a join code.
3. Each collaborator creates an **empty** folder and runs this there, and keeps it running. Joining replaces that folder's contents with the shared files:
    ```sh
    teamtypev join CODE
    ```
4. Everyone opens files from their shared folder in their editor, and starts their own preview (step 3).
5. Git: the sharer keeps the repository. Collaborators run `teamtypev git status`, `teamtypev git add FILE` and `teamtypev git commit -m "MESSAGE"`, which run in the sharer's repository. Alternatively, they run `source .teamtypev/activate` before starting their editor and then use plain `git`.

Details: [TeamTypeV getting started](https://github.com/SichangHe/TeamTypeV/blob/live-multi-editor/book/src/getting-started.md).

**Check**: text typed in one editor appears in the other's editor.

## Troubleshooting

- Preview blank, or "no PDF": read the error at the top of the preview, or open the LaTeX log (step 3).
- "lualatex not found": TeX Live is not on the PATH your editor sees. Start the editor from a terminal where `lualatex --version` works.
- No browser opened: the preview URL is shown as a message (in Neovim, `:messages`).
- Remove ltxp: `ltxp uninstall` lists everything the installer set up and removes it after asking (`--yes` skips the question).
- Bugs: [ltxp issues](https://github.com/SichangHe/latex_live_preview/issues), [TeamTypeV issues](https://github.com/SichangHe/TeamTypeV/issues).
