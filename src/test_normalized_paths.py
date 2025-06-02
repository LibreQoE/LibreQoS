#!/usr/bin/env python3
"""Test that both Python and Rust write to the normalized path"""

import os
import liblqos_python

lqos_dir = liblqos_python.get_libreqos_directory()
print(f"LQOS Directory: {lqos_dir}")

# Expected file paths
python_file = os.path.join(lqos_dir, "linux_tc.txt")
rust_file = os.path.join(lqos_dir, "tc-rust.txt")

print(f"\nExpected file paths:")
print(f"  Python: {python_file}")
print(f"  Rust:   {rust_file}")

# Clean up any old files
for f in [python_file, rust_file]:
    if os.path.exists(f):
        os.remove(f)
        print(f"\nRemoved old {os.path.basename(f)}")

# Test a simple bakery command
print("\nTesting bakery_add_structural_htb_class()...")
result = liblqos_python.bakery_add_structural_htb_class(
    interface="eth0",
    parent="1:",
    classid="1:10",
    rate_mbps=100.0,
    ceil_mbps=1000.0,
    site_hash=12345,
    r2q=10
)
print(f"Result: {result}")

# Check if file was created in the right place
if os.path.exists(rust_file):
    print(f"\n✓ Rust file created at: {rust_file}")
    with open(rust_file, "r") as f:
        print(f"  Contents: {f.read().strip()}")
else:
    print(f"\n✗ Rust file not found at: {rust_file}")
    
    # Check the old location
    old_location = "/home/herbert/Rust/LibreQoS/libreqos/LibreQoS/src/rust/lqosd/tc-rust.txt"
    if os.path.exists(old_location):
        print(f"  ⚠ File found at OLD location: {old_location}")
        print("  The bakery needs to be recompiled with the new path!")