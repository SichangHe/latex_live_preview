#!/bin/sh
# Installs llp from the latest GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
# Puts `llp` in ~/.local/bin, its LuaTeX module in ~/.local/lib/llp, and the Neovim plugin in
# Neovim's package path, where it starts itself for TeX files.
set -eu
REPO=SichangHe/latex_live_preview
PREFIX="${LLP_PREFIX:-$HOME/.local}"
NVIM_PACK="${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/llp/start/llp"

[ "$(uname -s)" = Linux ] || { echo "llp: only Linux is supported" >&2; exit 1; }
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "llp: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/llp.tar.gz" "https://github.com/$REPO/releases/latest/download/llp-$arch-linux.tar.gz"
tar -xzf "$tmp/llp.tar.gz" -C "$tmp"
mkdir -p "$PREFIX/bin" "$PREFIX/lib/llp" "$NVIM_PACK"
install -m 755 "$tmp/llp" "$PREFIX/bin/llp"
install -m 644 "$tmp/libllpfork.so" "$PREFIX/lib/llp/libllpfork.so"
rm -rf "$NVIM_PACK"
cp -r "$tmp/nvim" "$NVIM_PACK"

echo "llp installed: $PREFIX/bin/llp"
for cmd in lualatex synctex; do
	command -v "$cmd" >/dev/null || echo "llp: warning: $cmd not found; install TeX Live" >&2
done
case ":$PATH:" in
*":$PREFIX/bin:"*) ;;
*) echo "llp: add $PREFIX/bin to PATH" >&2 ;;
esac
