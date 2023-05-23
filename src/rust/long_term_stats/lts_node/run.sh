#!/bin/bash
pushd ../wasm_pipe
./build.sh
popd
pushd ../site_build
./esbuild.mjs
popd
pushd web
cp ../../site_build/output/* .
cp ../../site_build/src/main.html .
cp ../../site_build/wasm/wasm_pipe_bg.wasm .
popd
RUST_LOG=info RUST_BACKTRACE=1 cargo run
