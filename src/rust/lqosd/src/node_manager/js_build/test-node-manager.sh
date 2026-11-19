#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v node >/dev/null 2>&1; then
  echo "Missing required dependency: node (used only for built-in node_manager contract tests, no node_modules required)." >&2
  exit 1
fi

# Every `*.test.mjs` under src/ runs. The list is derived, not maintained: a
# hand-written list silently stops running a file the moment one is added (two
# topology-source contracts were edited by the i18n campaign and never executed),
# and the count it prints is the list's, not the tree's.
mapfile -t tests < <(find "${SCRIPT_DIR}/src" -name '*.test.mjs' | sort)

if (( ${#tests[@]} == 0 )); then
  echo "[test-node-manager] No *.test.mjs files found under ${SCRIPT_DIR}/src" >&2
  exit 1
fi

echo "[test-node-manager] Running ${#tests[@]} frontend contract test file(s)..."
node --test "${tests[@]}"

echo "[test-node-manager] Building bundles..."
"${SCRIPT_DIR}/esbuild.sh"

echo "[test-node-manager] Verifying node_manager build contract..."
"${SCRIPT_DIR}/test-build-contract.sh"
