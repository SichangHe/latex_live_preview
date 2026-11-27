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
escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
# `~/x` for paths under $HOME, so generated configs work on any machine with the same layout.
tilde() { case "$1" in "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }
# Lua expression for a path: `vim.fn.expand("~/x")` under $HOME, else a plain string.
lua_path() { printf 'vim.fn.expand("%s")' "$(escape "$(tilde "$1")")"; }
# Emacs Lisp expression for a path.
el_path() { printf '(expand-file-name "%s")' "$(escape "$(tilde "$1")")"; }
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
for f in libltxpfork.so libltxpfork.dylib; do
    [ ! -e "$work/pkg/$f" ] || mv -f "$work/pkg/$f" "$lib/$f"
done
mv -f "$work/pkg/ltxp" "$dir/ltxp"
chmod 755 "$dir/ltxp"
echo "Installed $("$dir/ltxp" --version) to $(tilde "$dir/ltxp")"
for cmd in lualatex synctex; do
    command -v "$cmd" >/dev/null 2>&1 || echo "Warning: $cmd not found; install TeX Live (or MacTeX on macOS)." >&2
done
case ":$PATH:" in
*":$dir:"*) ;;
*) echo "Note: $(tilde "$dir") is not in your PATH; add it, for example: export PATH=\"$(tilde "$dir" | sed 's|^~|$HOME|'):\$PATH\"" ;;
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

for editor in $editors; do
    case "$editor" in
    nvim)
        plugin="$HOME/.local/share/ltxp/nvim"
        rm -rf "$plugin"
        mkdir -p "$(dirname "$plugin")"
        cp -R "$work/pkg/nvim" "$plugin"
        # Earlier installers copied the plugin into Neovim's package dir; remove that copy.
        rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/${NVIM_APPNAME:-nvim}/site/pack/ltxp"
        config="${XDG_CONFIG_HOME:-$HOME/.config}/${NVIM_APPNAME:-nvim}"
        # The first module lazy.nvim imports specs from, e.g. `{ import = "plugins" }`.
        # Prefer one whose folder exists in this config; others (`lazyvim.plugins`) live in plugins.
        module=
        for m in $(grep -RhoEs "import[[:space:]]*=[[:space:]]*['\"][A-Za-z0-9_.-]+['\"]" "$config/init.lua" "$config/lua" |
            sed -E "s/.*['\"]([^'\"]+)['\"]/\1/"); do
            [ -n "$module" ] || case "$m" in lazyvim.*) ;; *) module=$m ;; esac
            if [ -d "$config/lua/$(printf '%s' "$m" | tr . /)" ]; then module=$m; break; fi
        done
        spec_file="$config/lua/$(printf '%s' "$module" | tr . /)/ltxp.lua"
        entry=$(printf '%s\n' '{' "  dir = $(lua_path "$plugin")," '  name = "ltxp",' \
            '  cmd = { "LtxpPreview", "LtxpStop" },' "  opts = { cmd = { $(lua_path "$dir/ltxp") } }," '},')
        spec=$(printf '%s\n%s\n%s\n%s\n' '-- Generated by the ltxp installer.' 'return {' "$(printf '%s\n' "$entry" | sed 's/^/  /')" '}')
        echo
        echo "Neovim:"
        echo "  plugin files: $(tilde "$plugin")"
        if [ -n "$module" ]; then
            if [ ! -e "$spec_file" ] || grep -Fqx -- '-- Generated by the ltxp installer.' "$spec_file"; then
                mkdir -p "$(dirname "$spec_file")"
                printf '%s\n' "$spec" >"$spec_file"
                echo "  lazy.nvim: wrote $(tilde "$spec_file")"
                echo "    (your lazy.nvim setup imports every spec in module \"$module\", so it picks this one up)"
                echo "    nothing runs until you call :LtxpPreview, which starts $(tilde "$dir/ltxp")"
                echo "  next: restart Neovim, open a .tex file, run :LtxpPreview"
            else
                echo "  lazy.nvim: $(tilde "$spec_file") already exists and is not from this installer; left unchanged"
                echo "  next: add this entry to the list it returns, then restart Neovim:"
                printf '%s\n' "$entry" | sed 's/^/    /'
            fi
        else
            echo "  lazy.nvim: not set up; found no \`import = \"...\"\` spec module in $(tilde "$config")"
            echo "  next, if you use lazy.nvim: add this entry to your plugin list, then restart Neovim:"
            printf '%s\n' "$entry" | sed 's/^/    /'
            echo "  next, without a plugin manager: add these lines to init.lua:"
            printf '    vim.opt.rtp:append(%s)\n    require("ltxp").setup({ cmd = { %s } })\n' "$(lua_path "$plugin")" "$(lua_path "$dir/ltxp")"
            echo "  then: open a .tex file and run :LtxpPreview"
        fi
        ;;
    emacs)
        plugin="$HOME/.local/share/ltxp/emacs"
        mkdir -p "$plugin"
        mv -f "$work/pkg/emacs/ltxp.el" "$plugin/ltxp.el"
        printf '(setq ltxp-command (list %s))\n(load "%s/ltxp.el")\n(ltxp-setup)\n' \
            "$(el_path "$dir/ltxp")" "$(escape "$(tilde "$plugin")")" >"$plugin/setup.el"
        init="$HOME/.emacs.d/init.el"
        if [ ! -e "$HOME/.emacs" ] && [ ! -e "$HOME/.emacs.d" ] && [ -d "${XDG_CONFIG_HOME:-$HOME/.config}/emacs" ]; then
            init="${XDG_CONFIG_HOME:-$HOME/.config}/emacs/init.el"
        elif [ -f "$HOME/.emacs.el" ]; then
            init="$HOME/.emacs.el"
        elif [ -f "$HOME/.emacs" ]; then
            init="$HOME/.emacs"
        fi
        mkdir -p "$(dirname "$init")"
        loader="(load \"$(escape "$(tilde "$plugin")")/setup.el\")"
        grep -Fqx "$loader" "$init" 2>/dev/null || printf '\n%s\n' "$loader" >>"$init"
        echo "Installed the Emacs plugin (needs Emacs 29+) and added its loader to $(tilde "$init")."
        echo "  next: restart Emacs, open a .tex file, run M-x ltxp-preview"
        ;;
    code | codium)
        if ! command -v "$editor" >/dev/null 2>&1; then
            echo "ltxp install: $editor command not found; enable the editor's shell command, then rerun with --editor $editor" >&2
            exit 1
        fi
        download "$assets/ltxp.vsix" "$work/ltxp.vsix"
        "$editor" --install-extension "$work/ltxp.vsix" --force
        [ "$dir" = "$HOME/.local/bin" ] || echo "Set the ltxp.path setting to $(tilde "$dir/ltxp")."
        echo "Installed the ltxp extension for $editor."
        echo "  next: restart $editor, open a .tex file, run the command \"ltxp: Open preview\""
        ;;
    esac
done
echo "Without an editor: ltxp main.tex"
