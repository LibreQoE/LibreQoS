#!/usr/bin/python3
"""
LibreQoS Test Runner

This script runs the LibreQoS test suite to prevent regressions and ensure
functionality works as expected. It's designed to be lightweight and focused
on critical functionality.

Usage:
    python3 run_tests.py                    # Run all tests
    python3 run_tests.py --fractional       # Run only fractional rate tests  
    python3 run_tests.py --ip               # Run only IP address tests
    python3 run_tests.py --verbose          # Run with verbose output
    python3 run_tests.py --quick            # Run only quick tests
"""

import unittest
import sys
import os
import argparse

def discover_and_run_tests(test_pattern="test_*.py", verbosity=1):
    """
    Discover and run tests matching the given pattern.
    """
    loader = unittest.TestLoader()
    start_dir = os.path.dirname(os.path.abspath(__file__))
    suite = loader.discover(start_dir, pattern=test_pattern)
    
    runner = unittest.TextTestRunner(verbosity=verbosity)
    result = runner.run(suite)
    
    return result.wasSuccessful()

def run_specific_test_file(test_file, verbosity=1):
    """
    Run tests from a specific test file.
    """
    loader = unittest.TestLoader()
    suite = loader.loadTestsFromName(test_file.replace('.py', ''))
    
    runner = unittest.TextTestRunner(verbosity=verbosity)
    result = runner.run(suite)
    
    return result.wasSuccessful()

def main():
    parser = argparse.ArgumentParser(description='LibreQoS Test Runner')
    parser.add_argument('--fractional', action='store_true',
                        help='Run only fractional rate tests')
    parser.add_argument('--ip', action='store_true', 
                        help='Run only IP address tests')
    parser.add_argument('--verbose', '-v', action='store_true',
                        help='Verbose output')
    parser.add_argument('--quick', action='store_true',
                        help='Run only quick tests (skip slow integration tests)')
    
    args = parser.parse_args()
    
    verbosity = 2 if args.verbose else 1
    
    print("=" * 60)
    print("LibreQoS Test Suite")
    print("=" * 60)
    
    # Determine which tests to run
    if args.fractional:
        print("Running fractional rate tests only...")
        success = run_specific_test_file('test_fractional_rates', verbosity)
    elif args.ip:
        print("Running IP address tests only...")
        success = run_specific_test_file('testIP', verbosity)
    elif args.quick:
        print("Running quick tests only...")
        # Run tests that are fast and don't require external dependencies
        test_files = ['test_fractional_rates', 'testIP']
        success = True
        for test_file in test_files:
            if not run_specific_test_file(test_file, verbosity):
                success = False
    else:
        print("Running all available tests...")
        success = discover_and_run_tests(verbosity=verbosity)
    
    print("\n" + "=" * 60)
    if success:
        print("✅ All tests passed!")
        print("\nRecommendations:")
        print("• Run tests after each significant change")
        print("• Add new tests when implementing new features")
        print("• Use --quick for rapid development feedback")
        return 0
    else:
        print("❌ Some tests failed!")
        print("\nTroubleshooting:")
        print("• Check the test output above for specific failures")
        print("• Ensure you're in the correct directory")
        print("• Verify all dependencies are installed")
        return 1

if __name__ == '__main__':
    sys.exit(main())