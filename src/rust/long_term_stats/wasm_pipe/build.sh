#!/bin/bash
cargo build --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir staging/ ../../target/wasm32-unknown-unknown/debug/wasm_pipe.wasm
cp staging/* ../site_build/wasm
cp staging/wasm_pipe_bg.wasm ../lts_node/web
