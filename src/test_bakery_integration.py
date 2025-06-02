#!/usr/bin/env python3
"""
Test script to verify bakery integration with LibreQoS.py

This script:
1. Runs LibreQoS.py (which will write to both linux_tc.txt and tc-rust.txt)
2. Compares the outputs to ensure they match
3. Reports any differences
"""

import os
import sys
import subprocess
import difflib

def run_libreqos():
    """Run LibreQoS.py and capture output"""
    print("Running LibreQoS.py...")
    try:
        result = subprocess.run(['python3', 'LibreQoS.py'], 
                              capture_output=True, text=True)
        if result.returncode != 0:
            print(f"LibreQoS.py failed with return code {result.returncode}")
            print(f"STDOUT:\n{result.stdout}")
            print(f"STDERR:\n{result.stderr}")
            return False
        print("LibreQoS.py completed successfully")
        return True
    except Exception as e:
        print(f"Error running LibreQoS.py: {e}")
        return False

def compare_tc_outputs():
    """Compare linux_tc.txt (Python) with tc-rust.txt (Rust)"""
    python_file = "linux_tc.txt"
    rust_file = "tc-rust.txt"
    
    # Check if both files exist
    if not os.path.exists(python_file):
        print(f"ERROR: {python_file} not found. LibreQoS.py may not have run correctly.")
        return False
    
    if not os.path.exists(rust_file):
        print(f"ERROR: {rust_file} not found. Bakery may not be running or not writing to file.")
        return False
    
    # Read both files
    with open(python_file, 'r') as f:
        python_lines = f.readlines()
    
    with open(rust_file, 'r') as f:
        rust_lines = f.readlines()
    
    print(f"\nPython generated {len(python_lines)} TC commands")
    print(f"Rust generated {len(rust_lines)} TC commands")
    
    # Compare line counts
    if len(python_lines) != len(rust_lines):
        print(f"\nWARNING: Different number of commands!")
        print(f"  Python: {len(python_lines)} commands")
        print(f"  Rust:   {len(rust_lines)} commands")
    
    # Find differences
    diff = list(difflib.unified_diff(
        python_lines, rust_lines,
        fromfile='linux_tc.txt (Python)',
        tofile='tc-rust.txt (Rust)',
        lineterm=''
    ))
    
    if diff:
        print("\nDifferences found between Python and Rust TC commands:")
        print("-" * 60)
        for line in diff[:50]:  # Show first 50 lines of diff
            print(line)
        if len(diff) > 50:
            print(f"... and {len(diff) - 50} more differences")
        print("-" * 60)
        return False
    else:
        print("\n✓ SUCCESS: Python and Rust TC commands are IDENTICAL!")
        return True

def check_prerequisites():
    """Check that necessary files and modules are available"""
    try:
        import liblqos_python
        print("✓ liblqos_python module found")
    except ImportError:
        print("✗ ERROR: liblqos_python module not found")
        print("  Make sure the Rust Python module is built and in the Python path")
        return False
    
    if not os.path.exists("LibreQoS.py"):
        print("✗ ERROR: LibreQoS.py not found in current directory")
        return False
    
    print("✓ LibreQoS.py found")
    
    # Clean up old TC output files
    for f in ["linux_tc.txt", "tc-rust.txt"]:
        if os.path.exists(f):
            os.remove(f)
            print(f"  Cleaned up old {f}")
    
    return True

def main():
    print("LibreQoS Bakery Integration Test")
    print("=" * 40)
    
    if not check_prerequisites():
        print("\nPrerequisites check failed. Please fix the issues above.")
        return 1
    
    print("\nNOTE: This test requires lqosd to be running!")
    print("      If you see connection errors, please start lqosd first.")
    input("\nPress Enter to continue...")
    
    # Run LibreQoS.py
    if not run_libreqos():
        print("\nLibreQoS.py execution failed")
        return 1
    
    # Compare outputs
    if compare_tc_outputs():
        print("\nIntegration test PASSED! 🎉")
        print("\nNext steps:")
        print("1. Verify the TC commands look correct")
        print("2. Try with different ShapedDevices.csv configurations")
        print("3. Test with fractional speeds")
        return 0
    else:
        print("\nIntegration test FAILED")
        print("\nTroubleshooting:")
        print("1. Check that bakery is configured to write to file (WRITE_TC_TO_FILE = true)")
        print("2. Verify both Python and Rust are using the same R2Q value")
        print("3. Check for any rounding differences in rate calculations")
        return 1

if __name__ == "__main__":
    sys.exit(main())