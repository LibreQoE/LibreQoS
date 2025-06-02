#!/usr/bin/env python3
"""
Debug script to analyze differences between Python and Rust TC outputs
"""

import os
import re

def analyze_tc_files():
    """Analyze TC command files for common issues"""
    python_file = "linux_tc.txt"
    rust_file = "tc-rust.txt"
    
    if not os.path.exists(python_file):
        print(f"ERROR: {python_file} not found")
        return
    
    if not os.path.exists(rust_file):
        print(f"ERROR: {rust_file} not found")
        return
    
    with open(python_file, 'r') as f:
        python_lines = f.readlines()
    
    with open(rust_file, 'r') as f:
        rust_lines = f.readlines()
    
    print(f"Python: {len(python_lines)} commands")
    print(f"Rust:   {len(rust_lines)} commands")
    print()
    
    # Count command types
    python_types = count_command_types(python_lines)
    rust_types = count_command_types(rust_lines)
    
    print("Command type counts:")
    print(f"{'Type':<20} {'Python':>10} {'Rust':>10} {'Diff':>10}")
    print("-" * 50)
    
    all_types = set(python_types.keys()) | set(rust_types.keys())
    for cmd_type in sorted(all_types):
        p_count = python_types.get(cmd_type, 0)
        r_count = rust_types.get(cmd_type, 0)
        diff = r_count - p_count
        print(f"{cmd_type:<20} {p_count:>10} {r_count:>10} {diff:>+10}")
    
    # Check for rate formatting differences
    print("\nRate formatting analysis:")
    analyze_rate_formats(python_lines, rust_lines)
    
    # Show first few lines from each
    print("\nFirst 5 commands from each file:")
    print("\nPython:")
    for i, line in enumerate(python_lines[:5]):
        print(f"  {i+1}: {line.strip()}")
    
    print("\nRust:")
    for i, line in enumerate(rust_lines[:5]):
        print(f"  {i+1}: {line.strip()}")

def count_command_types(lines):
    """Count TC command types"""
    types = {}
    for line in lines:
        parts = line.strip().split()
        if len(parts) >= 2:
            cmd_type = f"{parts[0]} {parts[1]}"
            types[cmd_type] = types.get(cmd_type, 0) + 1
    return types

def analyze_rate_formats(python_lines, rust_lines):
    """Analyze rate formatting differences"""
    # Pattern to find rate values
    rate_pattern = re.compile(r'rate\s+(\S+)')
    ceil_pattern = re.compile(r'ceil\s+(\S+)')
    
    python_rates = []
    rust_rates = []
    
    for line in python_lines:
        rates = rate_pattern.findall(line)
        ceils = ceil_pattern.findall(line)
        python_rates.extend(rates + ceils)
    
    for line in rust_lines:
        rates = rate_pattern.findall(line)
        ceils = ceil_pattern.findall(line)
        rust_rates.extend(rates + ceils)
    
    # Find unique rate formats
    python_formats = set()
    rust_formats = set()
    
    for rate in python_rates:
        if 'gbit' in rate:
            python_formats.add('gbit')
        elif 'mbit' in rate:
            python_formats.add('mbit')
        elif 'kbit' in rate:
            python_formats.add('kbit')
    
    for rate in rust_rates:
        if 'gbit' in rate:
            rust_formats.add('gbit')
        elif 'mbit' in rate:
            rust_formats.add('mbit')
        elif 'kbit' in rate:
            rust_formats.add('kbit')
    
    print(f"  Python rate formats: {python_formats}")
    print(f"  Rust rate formats:   {rust_formats}")
    
    # Sample some rates
    print("\n  Sample rates (first 10):")
    print(f"  Python: {python_rates[:10]}")
    print(f"  Rust:   {rust_rates[:10]}")

if __name__ == "__main__":
    print("TC Output Debug Analysis")
    print("=" * 50)
    analyze_tc_files()