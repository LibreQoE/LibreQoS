import importlib
import sys
import types
import unittest


def install_stubs():
    lqlib = types.ModuleType("liblqos_python")
    # Values used while integrationCommon defines NetworkNode defaults.
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
    lqlib.write_compiled_topology_from_python_graph_payload = lambda *a, **k: None
    lqlib.get_libreqos_directory = lambda: "/tmp/libreqos-device-ref-test"  # nosec B108
    lqlib.get_libreqos_state_directory = lambda: "/tmp/libreqos-device-ref-test/state"  # nosec B108
    lqlib.migrate_legacy_site_bandwidth_csv = lambda *a, **k: None
    lqlib.overrides_network_adjustments_materialized = lambda: []
    lqlib.splynx_api_key = lambda: ""
    lqlib.splynx_api_secret = lambda: ""
    lqlib.splynx_api_url = lambda: "http://example.invalid"
    lqlib.splynx_strategy = lambda: "full"
    lqlib.sonar_api_key = lambda: ""
    lqlib.sonar_api_url = lambda: "http://example.invalid"
    lqlib.snmp_community = lambda: "public"
    lqlib.sonar_airmax_ap_model_ids = lambda: []
    lqlib.sonar_ltu_ap_model_ids = lambda: []
    lqlib.sonar_active_status_ids = lambda: []
    lqlib.sonar_recurring_service_rates = lambda: []
    lqlib.sonar_recurring_excluded_service_names = lambda: []
    lqlib.sonar_strategy = lambda: "full"
    lqlib.visp_client_id = lambda: ""
    lqlib.visp_client_secret = lambda: ""
    lqlib.visp_username = lambda: ""
    lqlib.visp_password = lambda: ""
    lqlib.visp_isp_id = lambda: ""
    lqlib.visp_online_users_domain = lambda: ""
    lqlib.visp_timeout_secs = lambda: 60

    def _missing(name):
        return lambda *args, **kwargs: None

    lqlib.__getattr__ = _missing
    sys.modules["liblqos_python"] = lqlib


install_stubs()
integrationCommon = importlib.import_module("integrationCommon")
integrationSonar = importlib.import_module("integrationSonar")
integrationSplynx = importlib.import_module("integrationSplynx")
integrationVISP = importlib.import_module("integrationVISP")


def find_named(payload, name):
    if not isinstance(payload, dict):
        return None
    if name in payload and isinstance(payload[name], dict):
        return payload[name]
    for value in payload.values():
        found = find_named(value, name)
        if found is not None:
            return found
    return None


def strip_device_refs(value):
    if isinstance(value, dict):
        return {
            key: strip_device_refs(child)
            for key, child in value.items()
            if key != "device_ref"
        }
    if isinstance(value, list):
        return [strip_device_refs(child) for child in value]
    return value


class DeviceRefTests(unittest.TestCase):
    def build_graph(self, with_refs):
        net = integrationCommon.NetworkGraph()
        site = integrationCommon.NetworkNode(
            id="site-1",
            displayName="Site One",
            type=integrationCommon.NodeType.site,
            download=1000,
            upload=1000,
            networkJsonId="test:site:1",
        )
        ap = integrationCommon.NetworkNode(
            id="ap-1",
            displayName="AP One",
            parentId="site-1",
            type=integrationCommon.NodeType.ap,
            download=500,
            upload=500,
            networkJsonId="test:ap:1",
        )
        client = integrationCommon.NetworkNode(
            id="client-1",
            displayName="Client One",
            parentId="ap-1",
            type=integrationCommon.NodeType.client,
            download=100,
            upload=100,
        )
        device = integrationCommon.NetworkNode(
            id="device-1",
            displayName="Device One",
            parentId="client-1",
            type=integrationCommon.NodeType.device,
            ipv4=["192.0.2.10"],
        )
        for node in (site, ap, client, device):
            net.addRawNode(node)
        if with_refs:
            net.attachDeviceRef(
                ap,
                mgmt_ip="10.0.0.1",
                vendor_hint="test",
                model="Model",
                family_hint="family",
            )
        net.prepareTree()
        return net

    def test_refs_serialize_without_changing_nodes_edges_or_rates(self):
        plain = self.build_graph(with_refs=False)
        with_refs = self.build_graph(with_refs=True)

        plain_nodes = [
            (node.id, node.type, node.parentId, node.downloadMbps, node.uploadMbps)
            for node in plain.nodes
        ]
        ref_nodes = [
            (node.id, node.type, node.parentId, node.downloadMbps, node.uploadMbps)
            for node in with_refs.nodes
        ]
        self.assertEqual(plain_nodes, ref_nodes)
        self.assertEqual(plain._children_cache, with_refs._children_cache)

        plain_json = plain.buildNetworkJson()
        ref_json = with_refs.buildNetworkJson()
        self.assertEqual(strip_device_refs(ref_json), plain_json)

        ap_json = find_named(ref_json, "AP One")
        self.assertEqual(
            ap_json["device_ref"],
            {
                "node_id": "test:ap:1",
                "mgmt_ip": "10.0.0.1",
                "vendor_hint": "test",
                "model": "Model",
                "family_hint": "family",
            },
        )
        self.assertNotIn("device_ref", find_named(ref_json, "Site One"))

    def test_sonar_populates_device_ref(self):
        net = integrationCommon.NetworkGraph()
        integrationSonar.addInfrastructureNodes(
            net,
            sites=[{"id": "site_1", "name": "Site", "raw_id": "s1"}],
            aps=[
                {
                    "id": "ap_1",
                    "name": "AP",
                    "parent": "site_1",
                    "raw_id": "a1",
                    "ip": "10.0.0.5",
                    "model": 42,
                    "cpe_macs": ["aa:bb:cc:dd:ee:ff"],
                }
            ],
        )
        ap = net.nodes[net.findNodeIndexById("ap_1")]
        self.assertEqual(ap.device_ref.node_id, "sonar:ap:a1")
        self.assertEqual(ap.device_ref.mgmt_ip, "10.0.0.5")
        self.assertEqual(ap.device_ref.vendor_hint, "sonar")
        self.assertEqual(ap.device_ref.model, "42")
        self.assertIsNone(ap.device_ref.family_hint)
        site = net.nodes[net.findNodeIndexById("site_1")]
        self.assertIsNone(site.device_ref)
        net.prepareTree()
        self.assertEqual(find_named(net.buildNetworkJson(), "AP")["device_ref"]["mgmt_ip"], "10.0.0.5")

    def test_splynx_populates_device_ref(self):
        net = integrationCommon.NetworkGraph()
        integrationSplynx.createInfrastructureNodes(
            net,
            monitoring=[
                {
                    "id": 1,
                    "title": "Site",
                    "ip": "10.0.0.1",
                    "parent_id": None,
                },
                {
                    "id": 10,
                    "title": "AP",
                    "ip": "10.0.0.6",
                    "parent_id": 1,
                    "access_device": 1,
                },
            ],
            hardware_name={"1": "Site", "10": "AP"},
            hardware_parent={"10": 1},
            hardware_type={"1": "Site", "10": "AP"},
            siteBandwidth={},
            hardware_name_extended={"1": "Site", "10": "AP"},
        )
        ap = net.nodes[net.findNodeIndexById("10")]
        self.assertEqual(ap.device_ref.node_id, "splynx:ap:10")
        self.assertEqual(ap.device_ref.mgmt_ip, "10.0.0.6")
        self.assertEqual(ap.device_ref.vendor_hint, "splynx")
        self.assertIsNone(ap.device_ref.model)
        net.prepareTree()
        self.assertEqual(find_named(net.buildNetworkJson(), "AP")["device_ref"]["mgmt_ip"], "10.0.0.6")

    def test_visp_populates_device_ref(self):
        net = integrationCommon.NetworkGraph()
        node_id = integrationVISP._ensure_infrastructure_node(
            net,
            pg_by_id={
                10: {
                    "id": 10,
                    "parent_id": None,
                    "location_name": "Site",
                    "description": "AP",
                    "type": "AP",
                    "default_type": "AirMax",
                }
            },
            site_cache={},
            ap_cache={},
            equipment_id=10,
        )
        self.assertEqual(node_id, "visp_eq_10")
        ap = net.nodes[net.findNodeIndexById("visp_eq_10")]
        self.assertEqual(ap.device_ref.node_id, "visp:eq:10")
        # The production equipmentPg query does not select an IP: leave unset.
        self.assertIsNone(ap.device_ref.mgmt_ip)
        self.assertEqual(ap.device_ref.vendor_hint, "visp")
        self.assertEqual(ap.device_ref.model, "AP")
        self.assertEqual(ap.device_ref.family_hint, "AirMax")
        net.prepareTree()
        ref_json = find_named(net.buildNetworkJson(), "AP")["device_ref"]
        self.assertNotIn("mgmt_ip", ref_json)
        self.assertEqual(ref_json["model"], "AP")

    def test_attach_device_ref_rejects_non_infrastructure(self):
        net = integrationCommon.NetworkGraph()
        device = integrationCommon.NetworkNode(
            id="device-1",
            displayName="Device",
            type=integrationCommon.NodeType.device,
        )
        with self.assertRaises(ValueError):
            net.attachDeviceRef(device, mgmt_ip="10.0.0.8")

    def test_device_ref_normalizes_and_omits_absent_fields(self):
        ref = integrationCommon.DeviceRef(
            node_id="test:ap:1",
            mgmt_ip="  10.0.0.1  ",
            vendor_hint="  ",
            model="\t",
            family_hint="",
        )
        self.assertEqual(ref.mgmt_ip, "10.0.0.1")
        self.assertIsNone(ref.vendor_hint)
        self.assertIsNone(ref.model)
        self.assertIsNone(ref.family_hint)
        self.assertEqual(
            ref.to_dict(),
            {"node_id": "test:ap:1", "mgmt_ip": "10.0.0.1"},
        )

        invalid = integrationCommon.DeviceRef(
            node_id="test:ap:2",
            mgmt_ip="not-an-ip",
            model=" 42 ",
        )
        self.assertIsNone(invalid.mgmt_ip)
        self.assertEqual(invalid.model, "42")
        self.assertEqual(
            invalid.to_dict(),
            {"node_id": "test:ap:2", "model": "42"},
        )

    def test_shaped_devices_identical_with_and_without_refs(self):
        plain = self.build_graph(with_refs=False)
        with_refs = self.build_graph(with_refs=True)
        self.assertEqual(
            with_refs.buildShapedDevicesArtifacts(),
            plain.buildShapedDevicesArtifacts(),
        )

        ref_json = with_refs.buildNetworkJson()
        self.assertIn("device_ref", find_named(ref_json, "AP One"))
        self.assertNotIn("device_ref", find_named(ref_json, "Site One"))


if __name__ == "__main__":
    unittest.main()
