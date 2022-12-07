import unittest
import sys

class TestIP(unittest.TestCase):
    def test_ignore(self):
        """
        Test that we are correctly ignoring an IP address
        """
        sys.path.append('testdata/')
        from integrationCommon import isInIgnoredSubnets
        self.assertEqual(isInIgnoredSubnets("192.168.1.1"),True)

    def test_not_ignore(self):
        """
        Test that we are not ignoring an IP address
        """
        sys.path.append('testdata/')
        from integrationCommon import isInIgnoredSubnets
        self.assertEqual(isInIgnoredSubnets("10.0.0.1"),False)

    def test_allowed(self):
        """
        Test that we are correctly permitting an IP address
        """
        sys.path.append('testdata/')
        from integrationCommon import isInAllowedSubnets
        self.assertEqual(isInAllowedSubnets("100.64.1.1"),True)

    def test_not_allowed(self):
        """
        Test that we are correctly not permitting an IP address
        """
        sys.path.append('testdata/')
        from integrationCommon import isInAllowedSubnets
        self.assertEqual(isInAllowedSubnets("101.64.1.1"),False)

    def test_is_permitted(self):
        """
        Test the combined isIpv4Permitted function for true
        """
        sys.path.append('testdata/')
        from integrationCommon import isIpv4Permitted
        self.assertEqual(isIpv4Permitted("100.64.1.1"),True)

    def test_is_not_permitted(self):
        """
        Test the combined isIpv4Permitted function for false
        """
        sys.path.append('testdata/')
        from integrationCommon import isIpv4Permitted
        self.assertEqual(isIpv4Permitted("101.64.1.1"),False)

if __name__ == '__main__':
        unittest.main()
