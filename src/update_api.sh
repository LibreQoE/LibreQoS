#!/bin/bash

set -euo pipefail

# DEVELOPER TOOL ONLY — never run from a package maintainer script.
#
# Fetch and install a PREBUILT lqos_api binary into src/bin (or an alternate
# destination). Since the beta, the supported install path is the versioned
# `libreqos-api` apt package (docs/beta_api_packaging_plan.md); this script
# remains for developers who need a hand-installed binary.
#
# Because the package ships a versioned binary, this tool is deliberately
# strict: it requires an EXPLICIT versioned URL and the expected SHA-256, and
# refuses a floating/short/checksum-less download. It never installs packages
# and never touches apt.
#
# Usage:
#   update_api.sh --url URL --sha256 HEX [--bin-dir DIR] [--no-restart]

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

BIN_DIR="${LQOS_API_BIN_DIR:-$SCRIPT_DIR/bin}"
API_URL="${LQOS_API_URL:-}"
API_SHA256="${LQOS_API_SHA256:-}"
RESTART_SERVICE="${LQOS_API_RESTART_SERVICE:-1}"

usage() {
  cat <<EOF
Usage: $0 --url URL --sha256 HEX [--bin-dir DIR] [--no-restart]

A versioned URL and the expected SHA-256 are REQUIRED (no floating fetch).

Options:
  --url URL       Versioned download URL for the lqos_api zip (required)
  --sha256 HEX    Expected SHA-256 of the downloaded zip (required)
  --bin-dir DIR   Destination directory for lqos_api (default: $BIN_DIR)
  --no-restart    Do not restart libreqos-api.service

Environment:
  LQOS_API_BIN_DIR            Same as --bin-dir
  LQOS_API_URL                Same as --url
  LQOS_API_SHA256             Same as --sha256
  LQOS_API_RESTART_SERVICE    1 to restart libreqos-api (default), 0 to skip
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir)
      if [[ $# -lt 2 ]]; then echo "Error: --bin-dir requires a directory argument" >&2; usage; exit 2; fi
      BIN_DIR="$2"; shift 2 ;;
    --url)
      if [[ $# -lt 2 ]]; then echo "Error: --url requires a URL argument" >&2; usage; exit 2; fi
      API_URL="$2"; shift 2 ;;
    --sha256)
      if [[ $# -lt 2 ]]; then echo "Error: --sha256 requires a hex digest" >&2; usage; exit 2; fi
      API_SHA256="$2"; shift 2 ;;
    --no-restart)
      RESTART_SERVICE=0; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Error: Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$API_URL" ]]; then
  echo "Error: --url (or LQOS_API_URL) is required; this tool never uses a floating default." >&2
  usage
  exit 2
fi
if ! [[ "$API_SHA256" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "Error: --sha256 (or LQOS_API_SHA256) must be the full 64-hex digest." >&2
  usage
  exit 2
fi
command -v curl >/dev/null 2>&1 || { echo "Error: curl is required." >&2; exit 2; }
command -v unzip >/dev/null 2>&1 || { echo "Error: unzip is required (install it explicitly)." >&2; exit 2; }

TMP_ZIP="$(mktemp /tmp/lqos_api.XXXXXX.zip)"
trap 'rm -f "$TMP_ZIP"' EXIT

echo "Downloading lqos_api from $API_URL ..."
if ! curl -fsSL -o "$TMP_ZIP" "$API_URL"; then
  echo "lqos_api: NOT FETCHED — unreachable endpoint ($API_URL)" >&2
  exit 1
fi
ACTUAL_SHA256="$(sha256sum "$TMP_ZIP" | awk '{print $1}')"
if [[ "${ACTUAL_SHA256,,}" != "${API_SHA256,,}" ]]; then
  echo "lqos_api: CHECKSUM MISMATCH — expected ${API_SHA256,,}, got ${ACTUAL_SHA256}; refusing to install" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
echo "Extracting lqos_api into $BIN_DIR ..."
unzip -p "$TMP_ZIP" lqos_api > "$BIN_DIR/lqos_api.new"
chmod +x "$BIN_DIR/lqos_api.new"
mv "$BIN_DIR/lqos_api.new" "$BIN_DIR/lqos_api"

if [[ "$RESTART_SERVICE" == "1" ]]; then
  # The canonical unit name is libreqos-api (the old lqos_api name is gone).
  if /bin/systemctl cat libreqos-api.service >/dev/null 2>&1; then
    echo "Restarting libreqos-api.service (installed by the libreqos-api package)"
    sudo /bin/systemctl restart libreqos-api.service
  else
    echo "libreqos-api.service not found; skipping restart. Install the libreqos-api package for a managed service."
  fi
else
  echo "Skipping libreqos-api.service restart."
fi

echo "lqos_api: FETCHED ($(stat -c%s "$BIN_DIR/lqos_api" 2>/dev/null || echo '?') bytes) -> $BIN_DIR/lqos_api"
