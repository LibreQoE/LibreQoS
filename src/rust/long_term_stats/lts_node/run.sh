#!/bin/bash
pushd ../site_build
./esbuild.mjs
popd
RUST_LOG=warn RUST_BACKTRACE=1 cargo run
