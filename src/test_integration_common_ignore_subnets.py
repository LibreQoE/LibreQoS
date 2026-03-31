import csv
import importlib
import os
import sys
import tempfile
import types
import unittest


def install_common_stubs():
    lqlib = types.ModuleType("liblqos_python")
    lqlib.allowed_subnets = lambda: ["0.0.0.0/0", "::/0"]
    lqlib.ignore_subnets = lambda: []
    lqlib.generated_pn_download_mbps = lambda: 1000
    lqlib.generated_pn_upload_mbps = lambda: 1000
    lqlib.circuit_name_use_address = lambda: False
    lqlib.upstream_bandwidth_capacity_download_mbps = lambda: 1000
    lqlib.upstream_bandwidth_capacity_upload_mbps = lambda: 1000
    lqlib.find_ipv6_using_mikrotik = lambda: False
    lqlib.exclude_sites = lambda: []
    lqlib.bandwidth_overhead_factor = lambda: 1.0
    lqlib.committed_bandwidth_multiplier = lambda: 1.0
    lqlib.exception_cpes = lambda: []
    lqlib.promote_to_root_list = lambda: []
    lqlib.client_bandwidth_multiplier = lambda: 1.0
    sys.modules["liblqos_python"] = lqlib


install_common_stubs()
integrationCommon = importlib.import_module("integrationCommon")


class TestIntegrationCommonIgnoreSubnets(unittest.TestCase):
    def setUp(self):
        integrationCommon.allowed_subnets = lambda: ["0.0.0.0/0", "::/0"]
        integrationCommon.ignore_subnets = lambda: []

    def _build_client_with_device(self, ipv4=None, ipv6=None):
        net = integrationCommon.NetworkGraph()
        net.addRawNode(
            integrationCommon.NetworkNode(
                id="client-1",
                displayName="Client 1",
                type=integrationCommon.NodeType.client,
                download=100,
                upload=50,
            )
        )
        net.addRawNode(
            integrationCommon.NetworkNode(
                id="device-1",
                displayName="Device 1",
                type=integrationCommon.NodeType.device,
                parentId="client-1",
                ipv4=ipv4 or [],
                ipv6=ipv6 or [],
            )
        )
        return net

    def test_ignored_only_device_removes_entire_circuit(self):
        integrationCommon.ignore_subnets = lambda: ["100.64.0.0/10"]
        net = self._build_client_with_device(ipv4=["100.64.1.10/32"])

        net.prepareTree()

        self.assertEqual([node.id for node in net.nodes], ["FakeRoot"])

        with tempfile.TemporaryDirectory() as tmpdir:
            old_cwd = os.getcwd()
            try:
                os.chdir(tmpdir)
                net.createShapedDevices()
                with open("ShapedDevices.csv", newline="") as csvfile:
                    rows = list(csv.reader(csvfile))
            finally:
                os.chdir(old_cwd)

        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0][0], "Circuit ID")

    def test_mixed_device_ips_keep_only_permitted_addresses(self):
        integrationCommon.ignore_subnets = lambda: ["100.64.0.0/10"]
        net = self._build_client_with_device(
            ipv4=["100.64.1.10/32", "203.0.113.10/32"],
            ipv6=["2001:db8::10/128"],
        )

        net.prepareTree()

        self.assertEqual(len(net.nodes), 3)
        device = next(node for node in net.nodes if node.id == "device-1")
        self.assertEqual(device.ipv4, ["203.0.113.10/32"])
        self.assertEqual(device.ipv6, ["2001:db8::10/128"])

        with tempfile.TemporaryDirectory() as tmpdir:
            old_cwd = os.getcwd()
            try:
                os.chdir(tmpdir)
                net.createShapedDevices()
                with open("ShapedDevices.csv", newline="") as csvfile:
                    rows = list(csv.reader(csvfile))
            finally:
                os.chdir(old_cwd)

        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[1][0], "client-1")
        self.assertIn("203.0.113.10/32", rows[1][6])
        self.assertNotIn("100.64.1.10/32", rows[1][6])

    def test_prune_does_not_require_ip_to_be_in_allowed_subnets(self):
        integrationCommon.allowed_subnets = lambda: ["10.0.0.0/8"]
        integrationCommon.ignore_subnets = lambda: []
        net = self._build_client_with_device(ipv4=["203.0.113.10/32"])

        net.prepareTree()

        self.assertEqual(len(net.nodes), 3)
        device = next(node for node in net.nodes if node.id == "device-1")
        self.assertEqual(device.ipv4, ["203.0.113.10/32"])

        with tempfile.TemporaryDirectory() as tmpdir:
            old_cwd = os.getcwd()
            try:
                os.chdir(tmpdir)
                net.createShapedDevices()
                with open("ShapedDevices.csv", newline="") as csvfile:
                    rows = list(csv.reader(csvfile))
            finally:
                os.chdir(old_cwd)

        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[1][0], "client-1")
        self.assertIn("203.0.113.10/32", rows[1][6])


if __name__ == "__main__":
    unittest.main()
