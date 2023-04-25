#!/bin/bash
pushd ../site_build
./esbuild.mjs
popd
RUST_LOG=info cargo run
