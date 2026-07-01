#!/bin/sh
# straitjacket installer — downloads a prebuilt release binary for your platform.
#
#   curl -fsSL https://raw.githubusercontent.com/zmaril/straitjacket/main/install.sh | sh
#
# Env overrides:
#   STRAITJACKET_VERSION       release tag (default: latest)
#   STRAITJACKET_INSTALL_DIR   install directory (default: /usr/local/bin if writable, else ~/.local/bin)
set -eu

REPO="zmaril/straitjacket"
BIN="straitjacket"

info() { printf '%s\n' "straitjacket-install: $*" >&2; }
err() { info "error: $*"; exit 1; }

VERSION="${STRAITJACKET_VERSION:-latest}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *) err "unsupported OS '$os' — only Linux and macOS have prebuilt binaries. Build from source: cargo install --git https://github.com/${REPO}" ;;
esac
case "$arch" in
  x86_64 | amd64) arch_part="x86_64" ;;
  arm64 | aarch64) arch_part="aarch64" ;;
  *) err "unsupported architecture '$arch'" ;;
esac
target="${arch_part}-${os_part}"

# Only these targets are built by the release workflow.
case "$target" in
  x86_64-unknown-linux-gnu | x86_64-apple-darwin | aarch64-apple-darwin) : ;;
  *) err "no prebuilt binary for '$target' yet. Build from source: cargo install --git https://github.com/${REPO}" ;;
esac

asset="${BIN}-${target}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

if [ -n "${STRAITJACKET_INSTALL_DIR:-}" ]; then
  dir="$STRAITJACKET_INSTALL_DIR"
elif [ -w /usr/local/bin ]; then
  dir="/usr/local/bin"
else
  dir="$HOME/.local/bin"
fi
mkdir -p "$dir"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

info "downloading $url"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/$asset" || err "download failed — is $VERSION released for $target?"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/$asset" "$url" || err "download failed — is $VERSION released for $target?"
else
  err "need curl or wget to download"
fi

tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/$BIN" ] || err "archive did not contain '$BIN'"
chmod +x "$tmp/$BIN"
mv "$tmp/$BIN" "$dir/$BIN"
info "installed to $dir/$BIN"

case ":$PATH:" in
  *":$dir:"*) : ;;
  *) info "note: $dir is not on your PATH — add it, e.g. export PATH=\"$dir:\$PATH\"" ;;
esac

"$dir/$BIN" --version 2>/dev/null || true
