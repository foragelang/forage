#!/usr/bin/env sh
#
# Forage CLI installer.
#
#   curl -fsSL https://foragelang.com/install.sh | sh
#   curl -fsSL https://foragelang.com/install.sh | sh -s -- --to ~/bin
#   curl -fsSL https://foragelang.com/install.sh | sh -s -- --version v0.1.0
#
# Detects platform, fetches the matching release tarball from GitHub,
# verifies the bundled sha256, installs `forage` to ~/.local/bin (or the
# `--to` directory). Prints a PATH hint if needed.

set -eu

REPO="foragelang/forage"
DEST="${HOME}/.local/bin"
TAG="latest"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --to) DEST="$2"; shift 2;;
        --version) TAG="$2"; shift 2;;
        -h|--help)
            sed -n '2,16p' "$0"
            exit 0
            ;;
        *)
            echo "unknown flag: $1" >&2
            exit 1
            ;;
    esac
done

uname_s=$(uname -s)
uname_m=$(uname -m)

case "$uname_s" in
    Darwin) os=apple-darwin;;
    Linux)  os=unknown-linux-gnu;;
    MINGW*|MSYS*|CYGWIN*) os=pc-windows-msvc;;
    *) echo "unsupported OS: $uname_s" >&2; exit 1;;
esac

case "$uname_m" in
    x86_64|amd64) arch=x86_64;;
    arm64|aarch64) arch=aarch64;;
    *) echo "unsupported arch: $uname_m" >&2; exit 1;;
esac

target="${arch}-${os}"

case "$target" in
    aarch64-apple-darwin) ;;
    x86_64-apple-darwin) ;;
    aarch64-unknown-linux-gnu) ;;
    x86_64-unknown-linux-gnu) ;;
    x86_64-pc-windows-msvc) ;;
    *) echo "unsupported target: $target" >&2; exit 1;;
esac

if [ "$TAG" = "latest" ]; then
    TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | head -1 \
        | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
    if [ -z "$TAG" ]; then
        echo "could not resolve latest tag" >&2
        exit 1
    fi
fi

ext=tar.gz
[ "$os" = "pc-windows-msvc" ] && ext=zip

url="https://github.com/${REPO}/releases/download/${TAG}/forage-${TAG}-${target}.${ext}"
sha_url="https://github.com/${REPO}/releases/download/${TAG}/forage-${target}.sha256"

echo "==> Installing forage ${TAG} (${target}) to ${DEST}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

archive="$tmp/forage.${ext}"
echo "==> downloading $url"
curl -fsSL -o "$archive" "$url"

if curl -fsSL -o "$tmp/sha.txt" "$sha_url" 2>/dev/null; then
    expected=$(awk '{print $1; exit}' "$tmp/sha.txt")
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$archive" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$archive" | awk '{print $1}')
    else
        actual=""
    fi
    if [ -n "$actual" ] && [ -n "$expected" ] && [ "$actual" != "$expected" ]; then
        echo "checksum mismatch: expected $expected, got $actual" >&2
        exit 1
    fi
fi

mkdir -p "$DEST"
case "$ext" in
    tar.gz) tar -xzf "$archive" -C "$tmp";;
    zip)    (cd "$tmp" && unzip -q "$archive");;
esac

binary="$tmp/forage"
[ "$os" = "pc-windows-msvc" ] && binary="$tmp/forage.exe"
if [ ! -f "$binary" ]; then
    echo "extraction did not produce $binary" >&2
    exit 1
fi
chmod +x "$binary"
install_target="$DEST/$(basename "$binary")"
mv "$binary" "$install_target"

echo "==> installed: $install_target"

case ":$PATH:" in
    *":$DEST:"*)
        echo "==> $DEST is on PATH"
        ;;
    *)
        echo "==> add $DEST to your PATH:"
        if [ -n "${ZSH_VERSION:-}" ] || [ "$(basename "${SHELL:-/bin/sh}")" = "zsh" ]; then
            echo "      echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.zshrc"
        else
            echo "      echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.bashrc"
        fi
        ;;
esac

"$install_target" --version
