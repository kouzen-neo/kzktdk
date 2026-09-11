#!/usr/bin/env bash
# KZKT-DK (kzktdk) Universal Installer for Linux & macOS
# Usage: curl -fsSL https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/install.sh | bash

set -euo pipefail

REPO="kouzen-neo/kzktdk"
FALLBACK_TAG="v0.2.1"

# 1. Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        case "$ARCH" in
            x86_64|amd64)
                TARGET="x86_64-unknown-linux-gnu"
                EXT="tar.gz"
                ;;
            *)
                echo "[-] Unsupported architecture: $ARCH on Linux"
                echo "    Please build from source: https://github.com/$REPO#build-and-install"
                exit 1
                ;;
        esac
        ;;
    Darwin)
        case "$ARCH" in
            arm64|aarch64)
                TARGET="aarch64-apple-darwin"
                EXT="tar.gz"
                ;;
            x86_64|amd64)
                TARGET="x86_64-apple-darwin"
                EXT="tar.gz"
                ;;
            *)
                echo "[-] Unsupported architecture: $ARCH on macOS"
                exit 1
                ;;
        esac
        ;;
    *)
        echo "[-] Unsupported operating system: $OS"
        echo "    For Windows, run: irm https://raw.githubusercontent.com/$REPO/master/install.ps1 | iex"
        exit 1
        ;;
esac

echo "[*] Detected platform: $OS ($ARCH) -> Target: $TARGET"

LOCAL_ARCHIVE="${1:-}"

# 2. Resolve version and package
TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ASSET_NAME="kzktdk-${TARGET}.${EXT}"

if [ -n "$LOCAL_ARCHIVE" ] && [ -f "$LOCAL_ARCHIVE" ]; then
    echo "[*] Installing from local archive: $LOCAL_ARCHIVE"
    cp "$LOCAL_ARCHIVE" "$TMP_DIR/$ASSET_NAME"
else
    echo "[*] Fetching latest release info..."
    TAG=""
    if command -v curl >/dev/null 2>&1; then
        TAG="$(curl -sIL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null | rev | cut -d'/' -f1 | rev || true)"
    fi

    if [ -z "$TAG" ] || [ "$TAG" = "latest" ] || [ "$TAG" = "releases" ]; then
        TAG="$FALLBACK_TAG"
    fi

    DOWNLOAD_URL="https://github.com/$REPO/releases/download/${TAG}/${ASSET_NAME}"

    echo "[*] Installing KZKT-DK ${TAG}..."
    echo "[*] Download URL: $DOWNLOAD_URL"

    echo "[*] Downloading package..."
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --progress-bar "$DOWNLOAD_URL" -o "$TMP_DIR/$ASSET_NAME"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress "$DOWNLOAD_URL" -O "$TMP_DIR/$ASSET_NAME"
    else
        echo "[-] Error: curl or wget is required to download kzktdk."
        exit 1
    fi
fi

echo "[*] Extracting..."
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

# Find extracted root directory
SRC_DIR="$TMP_DIR"
if [ -d "$TMP_DIR/kzktdk-${TARGET}" ]; then
    SRC_DIR="$TMP_DIR/kzktdk-${TARGET}"
fi

# 4. Setup destinations
# Binary directory (~/.local/bin)
BIN_DIR="${HOME}/.local/bin"
if [ "$(id -u)" -eq 0 ]; then
    BIN_DIR="/usr/local/bin"
fi
mkdir -p "$BIN_DIR"

# Data directory (~/.local/share/kzktdk)
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/kzktdk"
mkdir -p "$DATA_DIR/models" "$DATA_DIR/fonts"

# Install binary
echo "[*] Installing binary to $BIN_DIR/kzktdk..."
mv "$SRC_DIR/kzktdk" "$BIN_DIR/kzktdk"
chmod +x "$BIN_DIR/kzktdk"

# Install models and fonts if packaged
if [ -d "$SRC_DIR/models" ]; then
    echo "[*] Installing models to $DATA_DIR/models/..."
    cp -rf "$SRC_DIR/models/"* "$DATA_DIR/models/" || true
fi

if [ -d "$SRC_DIR/fonts" ]; then
    echo "[*] Installing fonts to $DATA_DIR/fonts/..."
    cp -rf "$SRC_DIR/fonts/"* "$DATA_DIR/fonts/" || true
fi

echo ""
echo "========================================================="
echo "  KZKT-DK (kzktdk) successfully installed!"
echo "========================================================="
echo ""

# 5. Check PATH
case ":$PATH:" in
    *":$BIN_DIR:"*)
        echo "[+] $BIN_DIR is already in your PATH."
        ;;
    *)
        echo "[!] NOTICE: $BIN_DIR is not in your current PATH."
        echo "    Add it by running:"
        echo ""
        if [ -n "${ZSH_VERSION:-}" ] || [ "$(basename "${SHELL:-}")" = "zsh" ]; then
            echo '      echo '\''export PATH="'"$BIN_DIR"':$PATH"'\'' >> ~/.zshrc'
            echo '      source ~/.zshrc'
        else
            echo '      echo '\''export PATH="'"$BIN_DIR"':$PATH"'\'' >> ~/.bashrc'
            echo '      source ~/.bashrc'
        fi
        echo ""
        ;;
esac

echo "Run 'kzktdk --help' to get started!"
