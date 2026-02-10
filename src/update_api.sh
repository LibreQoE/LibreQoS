#!/bin/bash

set -euo pipefail

# Fetch and install the lqos_api binary into src/bin (or an alternate destination)
# - Downloads https://download.libreqos.com/api2.zip
# - Extracts the single file lqos_api
# - Places it into ./bin alongside other binaries (override with --bin-dir)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TMP_ZIP="$(mktemp /tmp/lqos_api.XXXXXX.zip)"

BIN_DIR="${LQOS_API_BIN_DIR:-$SCRIPT_DIR/bin}"
RESTART_SERVICE="${LQOS_API_RESTART_SERVICE:-1}"

usage() {
  cat <<EOF
Usage: $0 [--bin-dir DIR] [--no-restart]

Options:
  --bin-dir DIR   Destination directory for lqos_api (default: $SCRIPT_DIR/bin)
  --no-restart    Do not restart the lqos_api systemd service

Environment:
  LQOS_API_BIN_DIR            Same as --bin-dir
  LQOS_API_RESTART_SERVICE    1 to restart service (default), 0 to skip
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir)
      if [[ $# -lt 2 ]]; then
        echo "Error: --bin-dir requires a directory argument"
        usage
        exit 2
      fi
      BIN_DIR="$2"
      shift 2
      ;;
    --no-restart)
      RESTART_SERVICE=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Error: Unknown argument: $1"
      usage
      exit 2
      ;;
  esac
done

echo "Downloading lqos_api..."
if ! curl -fsSL -o "$TMP_ZIP" "https://download.libreqos.com/api2.zip"; then
  echo "Warning: Failed to download lqos_api; leaving existing binary untouched."
  rm -f "$TMP_ZIP"
  exit 0
fi

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

if [[ "$RESTART_SERVICE" == "1" ]]; then
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
else
  echo "Skipping lqos_api service restart."
fi

rm -f "$TMP_ZIP"
echo "lqos_api updated at $BIN_DIR/lqos_api"
