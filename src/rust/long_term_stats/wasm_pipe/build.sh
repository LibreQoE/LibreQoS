#!/bin/bash
CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_INCREMENTAL=false cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir staging/ ../../target/wasm32-unknown-unknown/release/wasm_pipe.wasm
cp staging/* ../site_build/wasm
cp staging/wasm_pipe_bg.wasm ../lts_node/web
