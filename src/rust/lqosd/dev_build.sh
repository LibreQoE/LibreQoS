#!/usr/bin/env bash
set -euo pipefail

# Tiny helper for local UI iteration: builds JS and copies it to bin/static2.
# Usage: from repo root or from this directory:
#   bash rust/lqosd/dev_build.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "[dev_build] Building JS bundles..."
pushd "$SCRIPT_DIR/src/node_manager/js_build" >/dev/null
./esbuild.sh
popd >/dev/null

echo "[dev_build] Copying bundles to bin/static2..."
mkdir -p "$SCRIPT_DIR/../../bin/static2"
cp -R "$SCRIPT_DIR/src/node_manager/js_build/out/"* "$SCRIPT_DIR/../../bin/static2/"

echo "[dev_build] Done. Hard refresh your browser (Ctrl/Cmd+Shift+R)."

