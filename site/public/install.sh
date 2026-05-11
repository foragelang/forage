#!/usr/bin/env bash
# Forage CLI installer.
#
#   curl -fsSL https://foragelang.com/install.sh | sh
#
# Env overrides:
#   FORAGE_INSTALL_DIR  install location (default: $HOME/.local/bin)
#   FORAGE_VERSION      tag to install (default: latest)

set -euo pipefail

OS="$(uname -s)"
ARCH="$(uname -m)"

if [ "$OS" != "Darwin" ]; then
    echo "forage: only macOS is supported at the moment (saw $OS)." >&2
    exit 1
fi

case "$ARCH" in
    arm64|x86_64) ;;
    *)
        echo "forage: unsupported arch $ARCH" >&2
        exit 1
        ;;
esac

REPO="foragelang/forage"
INSTALL_DIR="${FORAGE_INSTALL_DIR:-$HOME/.local/bin}"

if [ -n "${FORAGE_VERSION:-}" ]; then
    TAG="$FORAGE_VERSION"
else
    META_URL="https://api.github.com/repos/${REPO}/releases/latest"
    META=$(curl -fsSL "$META_URL")
    TAG=$(printf '%s' "$META" | sed -nE 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' | head -n1)
fi

if [ -z "$TAG" ]; then
    echo "forage: could not determine release tag." >&2
    exit 1
fi

ASSET="forage-${TAG}-macos.tar.gz"
URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"
SHA_URL="${URL}.sha256"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading forage ${TAG}…"
curl -fsSL "$URL" -o "$TMP/$ASSET"

# Verify sha256 if the sibling .sha256 file is present in the release.
if curl -fsSL "$SHA_URL" -o "$TMP/$ASSET.sha256" 2>/dev/null; then
    EXPECTED=$(awk '{print $1}' "$TMP/$ASSET.sha256")
    ACTUAL=$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')
    if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "forage: sha256 mismatch" >&2
        echo "  expected: $EXPECTED" >&2
        echo "  actual:   $ACTUAL" >&2
        exit 1
    fi
    echo "sha256 verified."
else
    echo "forage: no sha256 file in release; skipping verification." >&2
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP/$ASSET" -C "$TMP"
install -m 0755 "$TMP/forage" "$INSTALL_DIR/forage"

echo "Installed forage to $INSTALL_DIR/forage"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo
        echo "Note: $INSTALL_DIR is not on your PATH. Add it with:"
        echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc"
        ;;
esac

"$INSTALL_DIR/forage" --version || true
