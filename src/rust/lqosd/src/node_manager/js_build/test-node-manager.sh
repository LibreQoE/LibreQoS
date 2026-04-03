#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v node >/dev/null 2>&1; then
  echo "Missing required dependency: node (used only for built-in node_manager contract tests, no node_modules required)." >&2
  exit 1
fi

echo "[test-node-manager] Running frontend contract tests..."
node --test "${SCRIPT_DIR}/src/config/shaped_device_wire.test.mjs"

echo "[test-node-manager] Building bundles..."
"${SCRIPT_DIR}/esbuild.sh"

echo "[test-node-manager] Verifying node_manager build contract..."
"${SCRIPT_DIR}/test-build-contract.sh"
