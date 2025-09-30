#!/bin/bash

set -euo pipefail

# Fetch and install the lqos_api binary into src/bin
# - Downloads https://download.libreqos.com/api.zip
# - Extracts the single file lqos_api
# - Places it into ./bin alongside other binaries

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$SCRIPT_DIR/bin"
TMP_ZIP="$(mktemp /tmp/lqos_api.XXXXXX.zip)"

echo "Downloading lqos_api..."
curl -fsSL -o "$TMP_ZIP" "https://download.libreqos.com/api.zip"

# Ensure unzip is available
if ! command -v unzip >/dev/null 2>&1; then
  echo "'unzip' not found. Attempting to install it with sudo apt-get..."
  sudo apt-get update -y && sudo apt-get install -y unzip
fi

mkdir -p "$BIN_DIR"

echo "Extracting lqos_api into $BIN_DIR ..."
# Extract directly to a temporary file to avoid partial writes
unzip -p "$TMP_ZIP" lqos_api > "$BIN_DIR/lqos_api.new"
chmod +x "$BIN_DIR/lqos_api.new"
mv "$BIN_DIR/lqos_api.new" "$BIN_DIR/lqos_api"

rm -f "$TMP_ZIP"
echo "lqos_api updated at $BIN_DIR/lqos_api"

