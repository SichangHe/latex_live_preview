#!/bin/sh
# Installs ltxp from the latest GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/SichangHe/latex_live_preview/main/install.sh | sh
# Puts `ltxp` in ~/.local/bin, its LuaTeX module in ~/.local/lib/ltxp, and the Neovim plugin in
# Neovim's package path, where it starts itself for TeX files.
set -eu
REPO=SichangHe/latex_live_preview
PREFIX="${LTXP_PREFIX:-$HOME/.local}"
NVIM_PACK="${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/ltxp/start/ltxp"

[ "$(uname -s)" = Linux ] || { echo "ltxp: only Linux is supported" >&2; exit 1; }
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "ltxp: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/ltxp.tar.gz" "https://github.com/$REPO/releases/latest/download/ltxp-$arch-linux.tar.gz"
tar -xzf "$tmp/ltxp.tar.gz" -C "$tmp"
mkdir -p "$PREFIX/bin" "$PREFIX/lib/ltxp" "$NVIM_PACK"
install -m 755 "$tmp/ltxp" "$PREFIX/bin/ltxp"
install -m 644 "$tmp/libltxpfork.so" "$PREFIX/lib/ltxp/libltxpfork.so"
rm -rf "$NVIM_PACK"
cp -r "$tmp/nvim" "$NVIM_PACK"

echo "ltxp installed: $PREFIX/bin/ltxp"
for cmd in lualatex synctex; do
	command -v "$cmd" >/dev/null || echo "ltxp: warning: $cmd not found; install TeX Live" >&2
done
case ":$PATH:" in
*":$PREFIX/bin:"*) ;;
*) echo "ltxp: add $PREFIX/bin to PATH" >&2 ;;
esac
