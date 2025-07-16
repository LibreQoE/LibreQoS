#!/bin/bash
# Clean up TC output files for fresh testing

echo "Cleaning up TC output files..."

# Remove Python TC output
if [ -f "linux_tc.txt" ]; then
    rm linux_tc.txt
    echo "  ✓ Removed linux_tc.txt"
fi

# Remove Rust TC output
if [ -f "tc-rust.txt" ]; then
    rm tc-rust.txt
    echo "  ✓ Removed tc-rust.txt"
fi

echo "TC output files cleaned up!"