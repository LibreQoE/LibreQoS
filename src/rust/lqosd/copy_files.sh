#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
STATIC_SOURCE_DIR="${SCRIPT_DIR}/src/node_manager/static2"
JS_BUILD_DIR="${SCRIPT_DIR}/src/node_manager/js_build"
STATIC_TARGET_DIR="${SCRIPT_DIR}/../../bin/static2"

pushd "${JS_BUILD_DIR}" >/dev/null
./esbuild.sh
./test-build-contract.sh
popd >/dev/null

echo "Copying static"
rm -rf "${STATIC_TARGET_DIR:?}"
mkdir -p "${STATIC_TARGET_DIR}"
cp -v -R "${STATIC_SOURCE_DIR}/." "${STATIC_TARGET_DIR}/"
cp -R "${JS_BUILD_DIR}/out/." "${STATIC_TARGET_DIR}/"
echo "Done"
