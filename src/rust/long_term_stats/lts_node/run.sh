#!/bin/bash
pushd ../site_build
./esbuild.mjs
popd
pushd web
cp ../../site_build/output/* .
cp ../../site_build/src/main.html .
popd
RUST_LOG=warn RUST_BACKTRACE=1 cargo run
