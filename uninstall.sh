#!/usr/bin/env bash
# KZKT-DK (kzktdk) Uninstaller for Linux & macOS
# Usage: curl -fsSL https://raw.githubusercontent.com/kouzen-neo/kzktdk/master/uninstall.sh | bash
#    or: ./uninstall.sh [--purge]

set -euo pipefail

PURGE=false
for arg in "$@"; do
    if [ "$arg" = "--purge" ] || [ "$arg" = "-p" ]; then
        PURGE=true
    fi
done

echo "[*] Uninstalling KZKT-DK (kzktdk)..."

# 1. Remove binaries
REMOVED_BIN=false
for bin_path in "${HOME}/.local/bin/kzktdk" "${HOME}/.cargo/bin/kzktdk" "/usr/local/bin/kzktdk"; do
    if [ -f "$bin_path" ]; then
        if [ -w "$bin_path" ] || [ "$(id -u)" -eq 0 ]; then
            rm -f "$bin_path"
            echo "[+] Removed binary: $bin_path"
            REMOVED_BIN=true
        else
            echo "[!] Sudo required to remove $bin_path. Attempting with sudo..."
            sudo rm -f "$bin_path"
            echo "[+] Removed binary (sudo): $bin_path"
            REMOVED_BIN=true
        fi
    fi
done

if [ "$REMOVED_BIN" = false ]; then
    echo "[-] No kzktdk binary found in ~/.local/bin or /usr/local/bin."
fi

# 2. Remove data directory (models, fonts)
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/kzktdk"
if [ -d "$DATA_DIR" ]; then
    rm -rf "$DATA_DIR"
    echo "[+] Removed data directory: $DATA_DIR"
else
    echo "[-] No data directory found at $DATA_DIR."
fi

# 3. Purge cache and config if requested or default
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/kzktdk"
if [ "$PURGE" = true ] && [ -d "$CACHE_DIR" ]; then
    rm -rf "$CACHE_DIR"
    echo "[+] Purged cache directory: $CACHE_DIR"
fi

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/kzktdk"
if [ "$PURGE" = true ] && [ -d "$CONFIG_DIR" ]; then
    rm -rf "$CONFIG_DIR"
    echo "[+] Purged config directory: $CONFIG_DIR"
fi

echo ""
echo "========================================================="
echo "  KZKT-DK has been successfully uninstalled."
echo "========================================================="
if [ "$PURGE" = false ] && [ -d "$CACHE_DIR" ]; then
    echo "Note: Cache at $CACHE_DIR was preserved."
    echo "Run with '--purge' to delete cache as well."
fi
