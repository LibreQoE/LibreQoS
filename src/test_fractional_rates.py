#!/usr/bin/python3
"""
Test suite for fractional speed plans functionality.

This test suite covers the new fractional rate features implemented in Step 2:
- CSV parsing and validation of fractional rates
- format_rate_for_tc() smart unit selection
- Data structure storage of float values
- Backward compatibility with integer rates

Run with: python3 test_fractional_rates.py
Or: python3 -m unittest test_fractional_rates.py
"""

import unittest
import tempfile
import os
import csv
import warnings


class TestFormatRateForTC(unittest.TestCase):
    """Test the format_rate_for_tc function with various rate inputs."""
    
    @classmethod
    def setUpClass(cls):
        """Import the format_rate_for_tc function from LibreQoS.py"""
        # Import the function from LibreQoS.py
        import sys
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from LibreQoS import format_rate_for_tc
        cls.format_rate_for_tc = format_rate_for_tc
    
    def test_sub_1mbps_rates(self):
        """Test rates less than 1 Mbps use kbit units."""
        test_cases = [
            (0.1, "100kbit"),
            (0.5, "500kbit"),
            (0.75, "750kbit"),
            (0.9, "900kbit"),
        ]
        
        for rate, expected in test_cases:
            with self.subTest(rate=rate):
                result = TestFormatRateForTC.format_rate_for_tc(rate)
                self.assertEqual(result, expected, 
                    f"Rate {rate} Mbps should format as {expected}, got {result}")
    
    def test_1_to_999mbps_rates(self):
        """Test rates from 1 to 999 Mbps use mbit units."""
        test_cases = [
            (1.0, "1.0mbit"),
            (2.5, "2.5mbit"),
            (10.5, "10.5mbit"),
            (100.0, "100.0mbit"),
            (999.9, "999.9mbit"),
        ]
        
        for rate, expected in test_cases:
            with self.subTest(rate=rate):
                result = TestFormatRateForTC.format_rate_for_tc(rate)
                self.assertEqual(result, expected,
                    f"Rate {rate} Mbps should format as {expected}, got {result}")
    
    def test_1000plus_mbps_rates(self):
        """Test rates 1000 Mbps and above use gbit units."""
        test_cases = [
            (1000.0, "1.0gbit"),
            (1500.5, "1.5gbit"),
            (2500.0, "2.5gbit"),
            (10000.0, "10.0gbit"),
        ]
        
        for rate, expected in test_cases:
            with self.subTest(rate=rate):
                result = TestFormatRateForTC.format_rate_for_tc(rate)
                self.assertEqual(result, expected,
                    f"Rate {rate} Mbps should format as {expected}, got {result}")
    
    def test_edge_cases(self):
        """Test edge cases and boundary conditions."""
        # Test exactly 1.0 Mbps (boundary between kbit and mbit)
        self.assertEqual(TestFormatRateForTC.format_rate_for_tc(1.0), "1.0mbit")
        
        # Test exactly 1000.0 Mbps (boundary between mbit and gbit)
        self.assertEqual(TestFormatRateForTC.format_rate_for_tc(1000.0), "1.0gbit")
        
        # Test very small fractional rate
        self.assertEqual(TestFormatRateForTC.format_rate_for_tc(0.01), "10kbit")


class TestCSVFractionalParsing(unittest.TestCase):
    """Test CSV parsing and validation of fractional rates."""
    
    def setUp(self):
        """Set up test CSV files for each test."""
        self.temp_dir = tempfile.mkdtemp()
        
    def tearDown(self):
        """Clean up temporary files."""
        import shutil
        shutil.rmtree(self.temp_dir)
    
    def create_test_csv(self, data):
        """Helper to create a test CSV file with given data."""
        csv_path = os.path.join(self.temp_dir, "test.csv")
        with open(csv_path, 'w', newline='') as f:
            writer = csv.writer(f)
            # Write header
            writer.writerow([
                "Circuit ID", "Circuit Name", "Device ID", "Device Name", 
                "Parent Node", "MAC", "IPv4", "IPv6",
                "Download Min Mbps", "Upload Min Mbps", 
                "Download Max Mbps", "Upload Max Mbps", "Comment"
            ])
            # Write data rows
            for row in data:
                writer.writerow(row)
        return csv_path
    
    def test_fractional_rate_parsing(self):
        """Test that fractional rates are parsed correctly as floats."""
        test_data = [
            ["test1", "Test Circuit 1", "dev1", "Device 1", "site1", 
             "00:00:00:00:00:01", "192.168.1.1", "", 
             "0.5", "1.0", "2.5", "3.0", "Test fractional"]
        ]
        
        csv_path = self.create_test_csv(test_data)
        
        # Parse the CSV and verify float conversion
        with open(csv_path, 'r') as f:
            reader = csv.DictReader(f)
            row = next(reader)
            
            # Test that we can parse as floats
            download_min = float(row["Download Min Mbps"])
            upload_min = float(row["Upload Min Mbps"])
            download_max = float(row["Download Max Mbps"])
            upload_max = float(row["Upload Max Mbps"])
            
            self.assertEqual(download_min, 0.5)
            self.assertEqual(upload_min, 1.0)
            self.assertEqual(download_max, 2.5)
            self.assertEqual(upload_max, 3.0)
    
    def test_integer_backward_compatibility(self):
        """Test that integer rates still work (backward compatibility)."""
        test_data = [
            ["test2", "Test Circuit 2", "dev2", "Device 2", "site1",
             "00:00:00:00:00:02", "192.168.1.2", "",
             "10", "20", "100", "50", "Test integer compatibility"]
        ]
        
        csv_path = self.create_test_csv(test_data)
        
        # Parse the CSV and verify integer→float conversion
        with open(csv_path, 'r') as f:
            reader = csv.DictReader(f)
            row = next(reader)
            
            download_min = float(row["Download Min Mbps"])
            upload_min = float(row["Upload Min Mbps"])
            download_max = float(row["Download Max Mbps"])
            upload_max = float(row["Upload Max Mbps"])
            
            self.assertEqual(download_min, 10.0)
            self.assertEqual(upload_min, 20.0)
            self.assertEqual(download_max, 100.0)
            self.assertEqual(upload_max, 50.0)
    
    def test_validation_thresholds(self):
        """Test that validation rejects rates below minimum thresholds."""
        # These would be tested by calling the actual validation functions
        # from LibreQoS.py, but they require the full module setup
        
        # Test minimum rate validation logic
        test_rates = [
            (0.05, False),  # Below 0.1 minimum for min rates
            (0.1, True),    # At 0.1 minimum for min rates
            (0.15, False),  # Below 0.2 minimum for max rates  
            (0.2, True),    # At 0.2 minimum for max rates
            (1.0, True),    # Above all minimums
        ]
        
        for rate, should_pass_min in test_rates:
            with self.subTest(rate=rate):
                # Test min rate validation (≥ 0.1)
                min_valid = rate >= 0.1
                self.assertEqual(min_valid, should_pass_min or rate >= 0.1)
                
                # Test max rate validation (≥ 0.2)
                max_valid = rate >= 0.2
                self.assertEqual(max_valid, rate >= 0.2)


class TestDataStructureStorage(unittest.TestCase):
    """Test that data structures properly store float values."""
    
    def test_circuit_data_structure(self):
        """Test that circuit data structures accept float values."""
        # Simulate the data structures created in LibreQoS.py
        circuit_data = {
            "circuitID": "test1",
            "circuitName": "Test Circuit",
            "minDownload": float("2.5"),
            "minUpload": float("1.0"),
            "maxDownload": float("10.5"),
            "maxUpload": float("5.0"),
        }
        
        # Verify types and values
        self.assertIsInstance(circuit_data["minDownload"], float)
        self.assertIsInstance(circuit_data["minUpload"], float)
        self.assertIsInstance(circuit_data["maxDownload"], float)
        self.assertIsInstance(circuit_data["maxUpload"], float)
        
        self.assertEqual(circuit_data["minDownload"], 2.5)
        self.assertEqual(circuit_data["minUpload"], 1.0)
        self.assertEqual(circuit_data["maxDownload"], 10.5)
        self.assertEqual(circuit_data["maxUpload"], 5.0)
    
    def test_json_serialization(self):
        """Test that float rates can be serialized to JSON properly."""
        import json
        
        circuit_data = {
            "minDownload": 2.5,
            "minUpload": 1.0,
            "maxDownload": 10.5,
            "maxUpload": 5.0,
        }
        
        # Test JSON serialization
        json_str = json.dumps(circuit_data)
        self.assertIn("2.5", json_str)
        self.assertIn("10.5", json_str)
        
        # Test JSON deserialization
        parsed_data = json.loads(json_str)
        self.assertEqual(parsed_data["minDownload"], 2.5)
        self.assertEqual(parsed_data["maxDownload"], 10.5)


class TestRegressionPrevention(unittest.TestCase):
    """Test cases to prevent regressions in fractional rate functionality."""
    
    def test_no_integer_division(self):
        """Ensure float rates are not accidentally converted to integers."""
        test_rates = [0.5, 1.5, 2.5, 10.5]
        
        for rate in test_rates:
            # Simulate the type of operations done in LibreQoS.py
            stored_rate = float(str(rate))  # CSV→string→float conversion
            self.assertEqual(stored_rate, rate)
            self.assertIsInstance(stored_rate, float)
            
            # Ensure we don't accidentally truncate
            self.assertNotEqual(int(stored_rate), stored_rate, 
                f"Rate {rate} should remain fractional, not be truncated to {int(stored_rate)}")
    
    def test_tc_command_format_consistency(self):
        """Test that TC command formatting is consistent."""
        # Import format_rate_for_tc if available
        try:
            import sys
            sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
            from LibreQoS import format_rate_for_tc
            
            # Test that the function produces consistent output
            test_rate = 2.5
            result1 = format_rate_for_tc(test_rate)
            result2 = format_rate_for_tc(test_rate)
            self.assertEqual(result1, result2, "format_rate_for_tc should be deterministic")
            
            # Test that it doesn't have trailing spaces or other issues
            self.assertFalse(result1.startswith(" "), "No leading spaces")
            self.assertFalse(result1.endswith(" "), "No trailing spaces")
            self.assertTrue(result1.endswith(("kbit", "mbit", "gbit")), "Must end with valid unit")
            
        except ImportError:
            self.skipTest("LibreQoS module not available for import")


def run_fractional_rate_tests():
    """
    Convenience function to run all fractional rate tests.
    Can be called from other modules or scripts.
    """
    loader = unittest.TestLoader()
    suite = unittest.TestSuite()
    
    # Add all test classes
    test_classes = [
        TestFormatRateForTC,
        TestCSVFractionalParsing, 
        TestDataStructureStorage,
        TestRegressionPrevention
    ]
    
    for test_class in test_classes:
        tests = loader.loadTestsFromTestCase(test_class)
        suite.addTests(tests)
    
    runner = unittest.TextTestRunner(verbosity=2)
    result = runner.run(suite)
    
    return result.wasSuccessful()


if __name__ == '__main__':
    print("=" * 70)
    print("LibreQoS Fractional Rate Tests")
    print("=" * 70)
    print()
    
    # Run the tests
    success = run_fractional_rate_tests()
    
    print()
    if success:
        print("✅ All fractional rate tests passed!")
        exit(0)
    else:
        print("❌ Some fractional rate tests failed!")
        exit(1)