#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Copying static"
mkdir -p "$SCRIPT_DIR/../../bin/static2/"
cp -v -R "$SCRIPT_DIR/src/node_manager/static2/"* "$SCRIPT_DIR/../../bin/static2/"
echo "Done"

pushd "$SCRIPT_DIR/src/node_manager/js_build" >/dev/null
./esbuild.sh
popd >/dev/null

cp -R "$SCRIPT_DIR/src/node_manager/js_build/out/"* "$SCRIPT_DIR/../../bin/static2/"
