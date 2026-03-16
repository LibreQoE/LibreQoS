import importlib
import sys
import types
import unittest


def install_splynx_stubs():
    lqlib = types.ModuleType("liblqos_python")
    lqlib.allowed_subnets = lambda: ["0.0.0.0/0"]
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
    lqlib.splynx_api_key = lambda: ""
    lqlib.splynx_api_secret = lambda: ""
    lqlib.splynx_api_url = lambda: "http://example.invalid"
    lqlib.splynx_strategy = lambda: "flat"
    lqlib.overwrite_network_json_always = False
    sys.modules["liblqos_python"] = lqlib


install_splynx_stubs()
integrationSplynx = importlib.import_module("integrationSplynx")
integrationCommon = importlib.import_module("integrationCommon")


class TestIntegrationSplynxStableIds(unittest.TestCase):
    def test_stable_splynx_device_id_uses_service_id(self):
        self.assertEqual(
            integrationSplynx.stable_splynx_device_id(93),
            "splynx_service_93",
        )

    def test_stable_splynx_device_id_supports_future_equipment_suffix(self):
        self.assertEqual(
            integrationSplynx.stable_splynx_device_id(93, "5045"),
            "splynx_service_93_equipment_5045",
        )

    def test_create_client_and_device_uses_stable_device_id(self):
        net = integrationCommon.NetworkGraph()
        circuit_id = integrationSplynx.createClientAndDevice(
            net,
            serviceItem={
                "id": 93,
                "customer_id": 29547,
                "tariff_id": 32,
                "mac": "2CC81BBDC9AB, 00E04C687E6A",
            },
            cust_id_to_name={29547: "Charles Massey and Alex Soto"},
            downloadForTariffID={32: 330.0},
            uploadForTariffID={32: 330.0},
            parent_node_id="ap_244",
            ipv4_list=["66.185.224.210", "66.185.224.210/32"],
            ipv6_list=[],
        )

        self.assertEqual(circuit_id, 93)
        self.assertEqual(len(net.nodes), 3)
        client = net.nodes[1]
        device = net.nodes[2]
        self.assertEqual(client.id, 93)
        self.assertEqual(device.id, "splynx_service_93")
        self.assertEqual(device.parentId, 93)
        self.assertEqual(device.ipv4, ["66.185.224.210", "66.185.224.210/32"])

    def test_online_fallback_uses_stable_device_id(self):
        net = integrationCommon.NetworkGraph()
        matched = integrationSplynx.create_devices_from_online_for_unhandled_services(
            net,
            allServices=[
                {
                    "id": 93,
                    "customer_id": 29547,
                    "status": "active",
                    "tariff_id": 32,
                    "mac": "2CC81BBDC9AB",
                }
            ],
            service_ids_handled=[],
            customersOnline=[
                {
                    "service_id": 93,
                    "ipv4": "66.185.224.210",
                    "ipv6": "",
                }
            ],
            cust_id_to_name={29547: "Charles Massey and Alex Soto"},
            downloadForTariffID={32: 330.0},
            uploadForTariffID={32: 330.0},
            allocated_ipv4s={},
            allocated_ipv6s={},
            parent_selector=lambda service: "ap_244",
            device_by_service_id={},
        )

        self.assertEqual(matched, 1)
        self.assertEqual(len(net.nodes), 3)
        self.assertEqual(net.nodes[2].id, "splynx_service_93")
        self.assertEqual(net.nodes[2].ipv4, ["66.185.224.210"])


if __name__ == "__main__":
    unittest.main()
