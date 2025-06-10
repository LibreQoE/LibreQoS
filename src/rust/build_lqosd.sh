#!/bin/bash
# Build lqosd with proper package specification

set -e

echo "Building lqosd..."
cd /home/herbert/Rust/LibreQoS/libreqos/LibreQoS/src/rust

# Build the lqosd package specifically
cargo build --release --package lqosd

echo "Build complete!"
echo "Binary location: target/release/lqosd"