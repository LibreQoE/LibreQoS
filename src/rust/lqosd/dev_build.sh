#!/usr/bin/env bash
set -euo pipefail

# Tiny helper for local UI iteration: builds JS and copies it to bin/static2.
# Usage: from repo root or from this directory:
#   bash rust/lqosd/dev_build.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "[dev_build] Building JS bundles..."
pushd "$SCRIPT_DIR/src/node_manager/js_build" >/dev/null
./test-node-manager.sh
popd >/dev/null

echo "[dev_build] Copying static HTML/CSS assets to bin/static2..."
mkdir -p "$SCRIPT_DIR/../../bin/static2"
cp -R "$SCRIPT_DIR/src/node_manager/static2/"* "$SCRIPT_DIR/../../bin/static2/"
# Catalog sources are build inputs (see copy_files.sh): only the compiled
# `out/i18n/*.js` scripts are served, so a stale JSON copy would be a file the
# browser never reads.
find "$SCRIPT_DIR/../../bin/static2/i18n" -maxdepth 1 -name '*.json' -delete 2>/dev/null || true

echo "[dev_build] Copying bundles to bin/static2..."
cp -R "$SCRIPT_DIR/src/node_manager/js_build/out/"* "$SCRIPT_DIR/../../bin/static2/"

echo "[dev_build] Done. Hard refresh your browser (Ctrl/Cmd+Shift+R)."
