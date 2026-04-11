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
    lqlib.migrate_legacy_site_bandwidth_csv = lambda *_args, **_kwargs: None
    lqlib.overrides_network_adjustments_materialized = lambda: []
    lqlib.exclude_sites = lambda: []
    lqlib.bandwidth_overhead_factor = lambda: 1.0
    lqlib.committed_bandwidth_multiplier = lambda: 1.0
    lqlib.exception_cpes = lambda: []
    lqlib.promote_to_root_list = lambda: []
    lqlib.client_bandwidth_multiplier = lambda: 1.0
    lqlib.write_compiled_topology_from_python_graph_payload = lambda *_args, **_kwargs: None
    lqlib.splynx_api_key = lambda: ""
    lqlib.splynx_api_secret = lambda: ""
    lqlib.splynx_api_url = lambda: "http://example.invalid"
    lqlib.splynx_strategy = lambda: "flat"
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

    def test_ap_site_prefers_network_sites_topology_when_present(self):
        self.assertTrue(
            integrationSplynx.strategy_uses_network_sites_topology(
                "ap_site",
                [{"id": 1, "title": "Site A"}],
                [{"id": 5, "network_site_id": 1}],
            )
        )

    def test_full_preserves_monitoring_hierarchy_even_with_network_sites(self):
        self.assertFalse(
            integrationSplynx.strategy_uses_network_sites_topology(
                "full",
                [{"id": 1, "title": "Site A"}],
                [{"id": 5, "network_site_id": 1}],
            )
        )

    def test_full_mode_uses_stable_generated_unattached_site(self):
        self.assertEqual(
            integrationSplynx.splynx_generated_unattached_parent_id("full"),
            "splynx_generated_unattached_site",
        )
        self.assertEqual(
            integrationSplynx.splynx_generated_unattached_site_network_id(),
            "libreqos:generated:splynx:site:unattached",
        )

    def test_non_full_modes_do_not_force_generated_unattached_site(self):
        self.assertIsNone(integrationSplynx.splynx_generated_unattached_parent_id("ap_site"))
        self.assertIsNone(integrationSplynx.splynx_generated_unattached_parent_id("ap_only"))
        self.assertIsNone(integrationSplynx.splynx_generated_unattached_parent_id("flat"))

    def test_create_infrastructure_nodes_normalizes_parent_ids(self):
        net = integrationCommon.NetworkGraph()
        integrationSplynx.createInfrastructureNodes(
            net,
            monitoring=[
                {"id": 1, "gps": None},
                {"id": "2", "gps": None},
            ],
            hardware_name={"1": "Parent Site", "2": "Child AP"},
            hardware_parent={"2": 1},
            hardware_type={"1": "Site", "2": "AP"},
            siteBandwidth={},
            hardware_name_extended={"1": "Parent Site", "2": "Child AP"},
        )

        net.prepareTree()

        parent = next(node for node in net.nodes if node.id == "1")
        child = next(node for node in net.nodes if node.id == "2")
        self.assertEqual(parent.parentIndex, 0)
        self.assertEqual(child.parentId, "1")
        self.assertEqual(child.parentIndex, net.findNodeIndexById("1"))

    def test_find_best_parent_node_normalizes_service_and_sector_ids(self):
        parent_node_id, assignment_method = integrationSplynx.findBestParentNode(
            {"router_id": 10, "access_device": 0},
            hardware_name={"10": "Router A", "20": "Sector A"},
            ipForRouter={"10": "192.0.2.1"},
            sectorForRouter={"10": [{"id": 20}]},
        )
        self.assertEqual(parent_node_id, "10")
        self.assertEqual(assignment_method, "router_id")

        parent_node_id, assignment_method = integrationSplynx.findBestParentNode(
            {"router_id": 30, "access_device": 0},
            hardware_name={"20": "Sector A"},
            ipForRouter={"30": "192.0.2.2"},
            sectorForRouter={"30": [{"id": 20}]},
        )
        self.assertEqual(parent_node_id, "20")
        self.assertEqual(assignment_method, "sector_id")


if __name__ == "__main__":
    unittest.main()
