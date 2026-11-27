#!/bin/sh
# Installs ltxp from this project's GitHub releases:
#     curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
# Options, after `sh -s --` when piped:
#     --dir DIR      install the executable into DIR instead of ~/.local/bin
#     --release TAG  install release TAG instead of the latest
#     --editor NAME  set up nvim, emacs, code, or codium; repeat for more editors
#     --no-editor    skip the interactive editor menu
set -efu

repo=SichangHe/latex_live_preview
dir="$HOME/.local/bin"
release=latest
editors=
prompt=1
add_editor() {
    case "$1" in
    nvim | emacs | code | codium) ;;
    *) echo "ltxp install: unknown editor $1" >&2; exit 1 ;;
    esac
    case " $editors " in
    *" $1 "*) ;;
    *) editors="$editors $1" ;;
    esac
}
while [ $# -gt 0 ]; do
    case "$1" in
    --dir) dir=$2; shift 2 ;;
    --release) release=$2; shift 2 ;;
    --editor)
        prompt=0
        if [ "$2" = none ]; then editors=; else add_editor "$2"; fi
        shift 2
        ;;
    --no-editor) editors=; prompt=0; shift ;;
    -h | --help)
        echo "Usage: install.sh [--dir DIR] [--release TAG] [--editor NAME ... | --no-editor]"
        echo "Defaults: ~/.local/bin, latest release; editor menu when run in a terminal."
        echo "Editors: nvim, emacs, code (VS Code), codium (VSCodium), none."
        exit 0
        ;;
    *) echo "ltxp install: unknown option $1" >&2; exit 1 ;;
    esac
done
case "$(uname -s)/$(uname -m)" in
Linux/x86_64 | Linux/amd64) platform=x86_64-linux ;;
Linux/aarch64 | Linux/arm64) platform=aarch64-linux ;;
Darwin/arm64) platform=aarch64-macos ;;
Darwin/x86_64) platform=x86_64-macos ;;
*) echo "ltxp install: no prebuilt binary for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
if [ "$release" = latest ]; then
    assets="https://github.com/$repo/releases/latest/download"
else
    assets="https://github.com/$repo/releases/download/$release"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
download() {
    echo "Downloading $1"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    else
        wget -qO "$2" "$1"
    fi
}
download "$assets/ltxp-$platform.tar.gz" "$work/ltxp.tar.gz"
mkdir -p "$work/pkg"
tar -xzf "$work/ltxp.tar.gz" -C "$work/pkg"
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)
lib="$dir/../lib/ltxp"
mkdir -p "$lib"
lib=$(cd "$lib" && pwd)
# Replace rather than overwrite, so a running ltxp keeps its files.
for f in "$work"/pkg/libltxpfork.*; do mv -f "$f" "$lib/"; done
mv -f "$work/pkg/ltxp" "$dir/ltxp"
chmod 755 "$dir/ltxp"
echo "Installed $("$dir/ltxp" --version) to $dir/ltxp"
for cmd in lualatex synctex; do
    command -v "$cmd" >/dev/null 2>&1 || echo "Warning: $cmd not found; install TeX Live (or MacTeX on macOS)." >&2
done
case ":$PATH:" in
*":$dir:"*) ;;
*) echo "Note: $dir is not in your PATH; add it, for example: export PATH=\"$dir:\$PATH\"" ;;
esac

if [ "$prompt" = 1 ] && [ -t 1 ] && ( : </dev/tty ) 2>/dev/null; then
    exec 3<>/dev/tty
    printf '\nSet up editors (space-separated choices)? [1] Neovim [2] Emacs [3] VS Code [4] VSCodium [Enter] Skip: ' >&3
    read -r choices <&3 || choices=
    for choice in $choices; do
        case "$choice" in
        1) choice=nvim ;;
        2) choice=emacs ;;
        3) choice=code ;;
        4) choice=codium ;;
        esac
        add_editor "$choice"
    done
    exec 3>&-
fi

escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
for editor in $editors; do
    case "$editor" in
    nvim)
        plugin="${XDG_DATA_HOME:-$HOME/.local/share}/${NVIM_APPNAME:-nvim}/site/pack/ltxp/start/ltxp"
        rm -rf "$plugin"
        mkdir -p "$(dirname "$plugin")"
        cp -R "$work/pkg/nvim" "$plugin"
        printf 'require("ltxp").setup({ cmd = { "%s" } })\n' "$(escape "$dir/ltxp")" >"$plugin/plugin/00-ltxp-cmd.lua"
        echo "Installed the Neovim plugin to $plugin. Open a .tex file in Neovim to start the preview."
        echo "If your plugin manager disables native packages, add this local plugin (lazy.nvim):"
        printf '  { dir = "%s", name = "ltxp", lazy = false }\n' "$(escape "$plugin")"
        ;;
    emacs)
        plugin="$HOME/.local/share/ltxp/emacs"
        mkdir -p "$plugin"
        mv -f "$work/pkg/emacs/ltxp.el" "$plugin/ltxp.el"
        printf '(setq ltxp-command (list "%s"))\n(load "%s/ltxp.el")\n(ltxp-setup)\n' \
            "$(escape "$dir/ltxp")" "$(escape "$plugin")" >"$plugin/setup.el"
        init="$HOME/.emacs.d/init.el"
        if [ ! -e "$HOME/.emacs" ] && [ ! -e "$HOME/.emacs.d" ] && [ -d "${XDG_CONFIG_HOME:-$HOME/.config}/emacs" ]; then
            init="${XDG_CONFIG_HOME:-$HOME/.config}/emacs/init.el"
        elif [ -f "$HOME/.emacs.el" ]; then
            init="$HOME/.emacs.el"
        elif [ -f "$HOME/.emacs" ]; then
            init="$HOME/.emacs"
        fi
        mkdir -p "$(dirname "$init")"
        loader="(load \"$(escape "$plugin")/setup.el\")"
        grep -Fqx "$loader" "$init" 2>/dev/null || printf '\n%s\n' "$loader" >>"$init"
        echo "Installed the Emacs plugin (needs Emacs 29+) and added its loader to $init. Restart Emacs."
        ;;
    code | codium)
        if ! command -v "$editor" >/dev/null 2>&1; then
            echo "ltxp install: $editor command not found; enable the editor's shell command, then rerun with --editor $editor" >&2
            exit 1
        fi
        download "$assets/ltxp.vsix" "$work/ltxp.vsix"
        "$editor" --install-extension "$work/ltxp.vsix" --force
        [ "$dir" = "$HOME/.local/bin" ] || echo "Set the ltxp.path setting to $dir/ltxp."
        echo "Installed the ltxp extension for $editor. Restart the editor."
        ;;
    esac
done
echo "Open a .tex file in your editor, or run: ltxp main.tex"
