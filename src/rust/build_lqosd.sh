#!/bin/bash
# Build lqosd with proper package specification

set -e

echo "Building lqosd..."
cd /opt/libreqos/src/rust

# Build the lqosd package specifically
cargo build --release --package lqosd

# Build the node_manager JavaScript bundle (runbook §1.6: v3 dropped
# this step and fresh deploys shipped a JS-less dashboard) and place
# the bundles with the static pages they serve alongside.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${SCRIPT_DIR}/lqosd/src/node_manager/js_build/esbuild.sh"
cp "${SCRIPT_DIR}/lqosd/src/node_manager/js_build/out/"*.js \
   "${SCRIPT_DIR}/lqosd/src/node_manager/static2/"

echo "Build complete!"
echo "Binary location: target/release/lqosd"