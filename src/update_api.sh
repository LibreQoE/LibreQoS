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

# Function copied from build_rust.sh
service_exists() {
    local n=$1
    if [[ $(systemctl list-units --all -t service --full --no-legend "$n.service" | sed 's/^\s*//g' | cut -f1 -d' ') == $n.service ]]; then
        return 0
    else
        return 1
    fi
}

echo "Restart lqos_api service (if present)"
if service_exists lqos_api; then
    echo "lqos_api is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_api
else
    echo "lqos_api service not found; skipping restart."
fi

rm -f "$TMP_ZIP"
echo "lqos_api updated at $BIN_DIR/lqos_api"
