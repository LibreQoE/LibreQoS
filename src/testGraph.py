import os
import sys
import types
import unittest

_STUBBED_MODULES = ("liblqos_python",)
_ORIGINAL_MODULES = {name: sys.modules.get(name) for name in _STUBBED_MODULES}


def install_graph_stubs():
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
    lqlib.exception_cpes = lambda: {}
    lqlib.promote_to_root_list = lambda: []
    lqlib.client_bandwidth_multiplier = lambda: 1.0
    lqlib.write_compiled_topology_from_python_graph_payload = lambda *_args, **_kwargs: None
    lqlib.get_libreqos_directory = lambda: os.getcwd()
    sys.modules["liblqos_python"] = lqlib

def setUpModule():
    sys.modules.pop("integrationCommon", None)
    for name in _STUBBED_MODULES:
        sys.modules.pop(name, None)
    install_graph_stubs()


def tearDownModule():
    sys.modules.pop("integrationCommon", None)
    for name, module in _ORIGINAL_MODULES.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module

class TestGraph(unittest.TestCase):
    def test_empty_graph(self):
        """
        Test instantiation of the graph type
        """
        from integrationCommon import NetworkGraph
        graph = NetworkGraph()
        self.assertEqual(len(graph.nodes), 1) # There is an automatic root entry
        self.assertEqual(graph.nodes[0].id, "FakeRoot")

    def test_empty_node(self):
        """
        Test instantiation of the GraphNode type
        """
        from integrationCommon import NetworkNode, NodeType
        node = NetworkNode("test")
        self.assertEqual(node.type.value, NodeType.site.value)
        self.assertEqual(node.id, "test")
        self.assertEqual(node.parentIndex, 0)

    def test_node_types(self):
        """
        Test that the NodeType enum is working
        """
        from integrationCommon import NetworkNode, NodeType
        node = NetworkNode("Test", type = NodeType.root)
        self.assertEqual(node.type.value, NodeType.root.value)
        node = NetworkNode("Test", type = NodeType.site)
        self.assertEqual(node.type.value, NodeType.site.value)
        node = NetworkNode("Test", type = NodeType.ap)
        self.assertEqual(node.type.value, NodeType.ap.value)
        node = NetworkNode("Test", type = NodeType.client)
        self.assertEqual(node.type.value, NodeType.client.value)

    def test_add_raw_node(self):
        """
        Adds a single node to a graph to ensure add works
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site"))
        self.assertEqual(len(graph.nodes), 2)
        self.assertEqual(graph.nodes[1].type.value, NodeType.site.value)
        self.assertEqual(graph.nodes[1].parentIndex, 0)
        self.assertEqual(graph.nodes[1].id, "Site")

    def test_replace_root(self):
        """
        Test replacing the default root node with a specified node
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        node = NetworkNode("Test", type = NodeType.site)
        graph.replaceRootNode(node)
        self.assertEqual(graph.nodes[0].id, "Test")

    def add_child_by_named_parent(self):
        """
        Tests inserting a node with a named parent
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site"))
        graph.addNodeAsChild("site", NetworkNode("Client", type = NodeType.client))
        self.assertEqual(len(graph.nodes), 3)
        self.assertEqual(graph.nodes[2].parentIndex, 1)
        self.assertEqual(graph.nodes[0].parentIndex, 0)

    def test_add_child_by_named_parent_survives_reparent(self):
        """
        Ensures addNodeAsChild persists parent identity so later reparenting
        keeps the child attached to the intended parent.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site"))
        graph.addNodeAsChild("Site", NetworkNode("Client", type=NodeType.client))

        graph._NetworkGraph__reparentById()

        self.assertEqual(len(graph.nodes), 3)
        self.assertEqual(graph.nodes[2].parentId, "Site")
        self.assertEqual(graph.nodes[2].parentIndex, 1)

    def test_reparent_by_name(self):
        """
        Tests that re-parenting a tree by name is functional
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Missing Site", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        self.assertEqual(len(graph.nodes), 7) # Includes 1 for the fake root
        self.assertEqual(graph.nodes[1].parentIndex, 0) # Site 1 is off root
        self.assertEqual(graph.nodes[2].parentIndex, 0) # Site 2 is off root
        self.assertEqual(graph.nodes[3].parentIndex, 1) # Client 1 found Site 1
        self.assertEqual(graph.nodes[4].parentIndex, 1) # Client 2 found Site 1
        self.assertEqual(graph.nodes[5].parentIndex, 2) # Client 3 found Site 2
        self.assertEqual(graph.nodes[6].parentIndex, 0) # Client 4 didn't find "Missing Site" and goes to root

    def test_find_by_id(self):
        """
        Tests that finding a node by name succeeds or fails
        as expected.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        self.assertEqual(graph.findNodeIndexById("Site 1"), -1) # Test failure
        graph.addRawNode(NetworkNode("Site 1"))
        self.assertEqual(graph.findNodeIndexById("Site 1"), 1) # Test success

    def test_find_by_name(self):
        """
        Tests that finding a node by name succeeds or fails
        as expected.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        self.assertEqual(graph.findNodeIndexByName("Site 1"), -1) # Test failure
        graph.addRawNode(NetworkNode("Site 1", "Site X"))
        self.assertEqual(graph.findNodeIndexByName("Site X"), 1) # Test success

    def test_find_children(self):
        """
        Tests that finding children in the tree works,
        both for full and empty cases.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Missing Site", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        self.assertEqual(graph.findChildIndices(1), [3, 4])
        self.assertEqual(graph.findChildIndices(2), [5])
        self.assertEqual(graph.findChildIndices(3), [])

    def test_clients_with_children(self):
        """
        Tests handling cases where a client site
        itself has children. This is only useful for
        relays where a site hasn't been created in the
        middle - but it allows us to graph the more
        pathological designs people come up with.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Client 3", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        graph._NetworkGraph__promoteClientsWithChildren()
        self.assertEqual(graph.nodes[5].type, NodeType.clientWithChildren)
        self.assertEqual(graph.nodes[6].type, NodeType.client) # Test that a client is still a client

    def test_client_with_children_promotion(self):
        """
        Test locating a client site with children, and then promoting it to
        create a generated site
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Client 3", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        graph._NetworkGraph__promoteClientsWithChildren()
        graph._NetworkGraph__clientsWithChildrenToSites()
        self.assertEqual(graph.nodes[5].type, NodeType.client)
        self.assertEqual(graph.nodes[6].type, NodeType.client) # Test that a client is still a client
        self.assertEqual(graph.nodes[7].type, NodeType.site)
        self.assertEqual(graph.nodes[7].id, "Client 3_gen")

    def test_generated_site_keeps_original_parent(self):
        """
        Ensures generated sites created from relay-style clients retain the
        original non-root parent after the reparent pass.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Parent Site"))
        graph.addRawNode(NetworkNode("Relay Client", parentId="Parent Site", type=NodeType.client))
        graph.addRawNode(NetworkNode("Child Client", parentId="Relay Client", type=NodeType.client))

        graph._NetworkGraph__reparentById()
        graph._NetworkGraph__promoteClientsWithChildren()
        graph._NetworkGraph__clientsWithChildrenToSites()

        generated_site = next(node for node in graph.nodes if node.id == "Relay Client_gen")
        self.assertEqual(generated_site.parentId, "Parent Site")
        self.assertEqual(generated_site.parentIndex, 1)

    def test_prepare_tree_lifts_nested_generated_sites_out_of_clients(self):
        """
        Nested generated sites should remain reachable from exported topology
        instead of being stranded below plain client nodes.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("AP 1", type=NodeType.ap))
        graph.addRawNode(NetworkNode("Bonnie McBride", parentId="AP 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("LeRoy Dozois", parentId="Bonnie McBride", type=NodeType.client))
        graph.addRawNode(NetworkNode("Nested Site", parentId="LeRoy Dozois", type=NodeType.site))
        graph.addRawNode(
            NetworkNode(
                "bonnie-device",
                parentId="Bonnie McBride",
                type=NodeType.device,
                ipv4=["100.64.0.1"],
            )
        )
        graph.addRawNode(
            NetworkNode(
                "leroy-device",
                parentId="LeRoy Dozois",
                type=NodeType.device,
                ipv4=["100.64.0.2"],
            )
        )

        graph.prepareTree()

        bonnie = next(node for node in graph.nodes if node.id == "Bonnie McBride")
        bonnie_site = next(node for node in graph.nodes if node.id == "Bonnie McBride_gen")
        leroy_site = next(node for node in graph.nodes if node.id == "LeRoy Dozois_gen")

        self.assertEqual(bonnie.type, NodeType.client)
        self.assertEqual(bonnie.parentId, "Bonnie McBride_gen")
        self.assertEqual(bonnie_site.parentId, "AP 1")
        self.assertEqual(leroy_site.parentId, "Bonnie McBride_gen")

    def test_find_unconnected(self):
        """
        Tests traversing a tree and finding nodes that
        have no connection to the rest of the tree.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Client 3", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        graph._NetworkGraph__promoteClientsWithChildren()
        graph.nodes[6].parentIndex = 6 # Create a circle
        unconnected = graph._NetworkGraph__findUnconnectedNodes()
        self.assertEqual(len(unconnected), 1)
        self.assertEqual(unconnected[0], 6)
        self.assertEqual(graph.nodes[unconnected[0]].id, "Client 4")

    def test_reconnect_unconnected(self):
        """
        Tests traversing a tree and finding nodes that
        have no connection to the rest of the tree.
        Reconnects them and ensures that the orphan is now
        parented.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Client 3", type=NodeType.client))
        graph._NetworkGraph__reparentById()
        graph._NetworkGraph__promoteClientsWithChildren()
        graph.nodes[6].parentIndex = 6 # Create a circle
        graph._NetworkGraph__reconnectUnconnected()
        unconnected = graph._NetworkGraph__findUnconnectedNodes()
        self.assertEqual(len(unconnected), 0)
        self.assertEqual(graph.nodes[6].parentIndex, 0)

    def test_network_json_exists(self):
        from integrationCommon import NetworkGraph
        import os
        if os.path.exists("network.json"):
            os.remove("network.json")
        graph = NetworkGraph()
        self.assertEqual(graph.doesNetworkJsonExist(), False)
        with open('network.json', 'w') as f:
            f.write('Dummy')
        self.assertEqual(graph.doesNetworkJsonExist(), True)
        os.remove("network.json")

    def test_network_json_example(self):
        """
        Rebuilds the network in network.example.json
        and makes sure that it matches.
        Should serve as an example for how an integration
        can build a functional tree.
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        import json
        net = NetworkGraph()
        net.addRawNode(NetworkNode("Site_1", "Site_1", "", NodeType.site, 1000, 1000))
        net.addRawNode(NetworkNode("Site_2", "Site_2", "", NodeType.site, 500, 500))
        net.addRawNode(NetworkNode("AP_A", "AP_A", "Site_1", NodeType.ap, 500, 500))
        net.addRawNode(NetworkNode("Site_3", "Site_3", "Site_1", NodeType.site, 500, 500))
        net.addRawNode(NetworkNode("PoP_5", "PoP_5", "Site_3", NodeType.site, 200, 200))        
        net.addRawNode(NetworkNode("AP_9", "AP_9", "PoP_5", NodeType.ap, 120, 120))
        net.addRawNode(NetworkNode("PoP_6", "PoP_6", "PoP_5", NodeType.site, 60, 60))
        net.addRawNode(NetworkNode("AP_11", "AP_11", "PoP_6", NodeType.ap, 30, 30))
        net.addRawNode(NetworkNode("PoP_1", "PoP_1", "Site_2", NodeType.site, 200, 200))
        net.addRawNode(NetworkNode("AP_7", "AP_7", "PoP_1", NodeType.ap, 100, 100))
        net.addRawNode(NetworkNode("AP_1", "AP_1", "Site_2", NodeType.ap, 150, 150))
        net.prepareTree()
        net.createNetworkJson()
        with open('network.json') as file:
            newFile = json.load(file)
        with open('network.example.json') as file:
            exampleFile = json.load(file)
        self.assertEqual(newFile, exampleFile)

    def test_network_json_writes_optional_node_ids(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        import json

        net = NetworkGraph()
        net.addRawNode(
            NetworkNode(
                "Site_1",
                "Site_1",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="sonar:site:123",
            )
        )
        net.addRawNode(
            NetworkNode(
                "AP_1",
                "AP_1",
                "Site_1",
                NodeType.ap,
                500,
                500,
                networkJsonId="sonar:ap:456",
            )
        )
        net.prepareTree()
        net.createNetworkJson()
        with open('network.json') as file:
            newFile = json.load(file)

        self.assertEqual(newFile["Site_1"]["id"], "sonar:site:123")
        self.assertEqual(newFile["Site_1"]["children"]["AP_1"]["id"], "sonar:ap:456")

    def test_ipv4_to_ipv6_map(self):
        """
        Tests the underlying functionality of finding an IPv6 address from an IPv4 mapping
        """
        from integrationCommon import NetworkGraph
        net = NetworkGraph()
        ipv4 = [ "100.64.1.1" ]
        ipv6 = []
        # Test that it doesn't cause issues without any mappings
        net._NetworkGraph__addIpv6FromMap(ipv4, ipv6)
        self.assertEqual(len(ipv4), 1)
        self.assertEqual(len(ipv6), 0)

        # Test a mapping
        net.ipv4ToIPv6 = {
            "100.64.1.1":"dead::beef/64"
        }
        net._NetworkGraph__addIpv6FromMap(ipv4, ipv6)
        self.assertEqual(len(ipv4), 1)
        self.assertEqual(len(ipv6), 1)
        self.assertEqual(ipv6[0], "dead::beef/64")

    def test_apply_client_bandwidth_multiplier_uses_higher_factor(self):
        import integrationCommon

        old_overhead = integrationCommon.bandwidth_overhead_factor
        old_multiplier = integrationCommon.client_bandwidth_multiplier
        try:
            integrationCommon.bandwidth_overhead_factor = lambda: 1.1
            integrationCommon.client_bandwidth_multiplier = lambda: 1.25
            self.assertEqual(integrationCommon.apply_client_bandwidth_multiplier(100), 125.0)

            integrationCommon.bandwidth_overhead_factor = lambda: 1.4
            integrationCommon.client_bandwidth_multiplier = lambda: 1.25
            self.assertEqual(integrationCommon.apply_client_bandwidth_multiplier(100), 140.0)
        finally:
            integrationCommon.bandwidth_overhead_factor = old_overhead
            integrationCommon.client_bandwidth_multiplier = old_multiplier

    def test_create_shaped_devices_preserves_effective_client_rate(self):
        import csv
        import os
        import tempfile
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        net = NetworkGraph()
        net.addRawNode(NetworkNode("client_1", "Client 1", "", NodeType.client, 150.0, 75.0))
        net.addRawNode(NetworkNode("device_1", "Device 1", "client_1", NodeType.device, ipv4=["100.64.1.10"], mac="AA:BB:CC:DD:EE:FF"))
        net.prepareTree()

        old_cwd = os.getcwd()
        with tempfile.TemporaryDirectory() as tmpdir:
            try:
                os.chdir(tmpdir)
                net.createShapedDevices()
                with open("ShapedDevices.csv", newline="") as csvfile:
                    rows = list(csv.reader(csvfile))
            finally:
                os.chdir(old_cwd)

        self.assertEqual(rows[1][10], "1")
        self.assertEqual(rows[1][11], "1")
        self.assertEqual(rows[1][12], "150.0")
        self.assertEqual(rows[1][13], "75.0")

    def test_site_exclusion(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        net = NetworkGraph()
        net.excludeSites = ['Site_2']
        net.addRawNode(NetworkNode("Site_1", "Site_1", "", NodeType.site, 1000, 1000))
        net.addRawNode(NetworkNode("Site_2", "Site_2", "", NodeType.site, 500, 500))
        self.assertEqual(len(net.nodes), 2)

    def test_site_exception(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        net = NetworkGraph()
        net.exceptionCPEs = {
            "Site_2": "Site_1"
        }
        net.addRawNode(NetworkNode("Site_1", "Site_1", "", NodeType.site, 1000, 1000))
        net.addRawNode(NetworkNode("Site_2", "Site_2", "", NodeType.site, 500, 500))
        self.assertEqual(net.nodes[2].parentId, "Site_1")
        net.prepareTree()
        self.assertEqual(net.nodes[2].parentIndex, 1)

    def test_native_topology_editor_uses_nearest_real_infrastructure_parent(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        net = NetworkGraph()
        net.addRawNode(
            NetworkNode(
                "site_1",
                "Site 1",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:1",
            )
        )
        net.addRawNode(
            NetworkNode(
                "relay_client",
                "Relay Client",
                "site_1",
                NodeType.client,
                100,
                100,
                networkJsonId="splynx:circuit:relay",
            )
        )
        net.addRawNode(
            NetworkNode(
                "child_site",
                "Child Site",
                "relay_client",
                NodeType.site,
                500,
                500,
                networkJsonId="splynx:site:2",
            )
        )
        net.addRawNode(
            NetworkNode(
                "relay_device",
                "Relay Device",
                "relay_client",
                NodeType.device,
                ipv4=["100.64.0.10"],
            )
        )

        net.prepareTree()
        editor = net.buildNativeTopologyEditorState("python/splynx")
        nodes = {node["node_id"]: node for node in editor["nodes"]}

        self.assertEqual(set(nodes.keys()), {"splynx:site:1", "splynx:site:2"})
        self.assertIsNone(nodes["splynx:site:1"]["current_parent_node_id"])
        self.assertFalse(nodes["splynx:site:1"]["can_move"])
        self.assertEqual(nodes["splynx:site:1"]["allowed_parents"], [])
        self.assertEqual(nodes["splynx:site:2"]["current_parent_node_id"], "splynx:site:1")
        self.assertEqual(nodes["splynx:site:2"]["current_parent_node_name"], "Site 1")
        self.assertFalse(nodes["splynx:site:2"]["can_move"])
        self.assertEqual(nodes["splynx:site:2"]["allowed_parents"], [])

    def test_legacy_parent_candidates_skip_fixed_roots(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        net = NetworkGraph()
        net.addRawNode(
            NetworkNode(
                "site_1",
                "Site 1",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:1",
            )
        )
        net.addRawNode(
            NetworkNode(
                "site_2",
                "Site 2",
                "site_1",
                NodeType.site,
                500,
                500,
                networkJsonId="splynx:site:2",
            )
        )

        net.prepareTree()
        candidates = net.buildTopologyParentCandidates()
        self.assertEqual(candidates["nodes"], [])

    def test_native_topology_editor_exposes_bounded_local_move_candidates(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        net = NetworkGraph()
        net.addRawNode(
            NetworkNode(
                "root_site",
                "Root Site",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:root",
            )
        )
        net.addRawNode(
            NetworkNode(
                "parent_a",
                "Parent A",
                "root_site",
                NodeType.site,
                800,
                800,
                networkJsonId="splynx:site:a",
            )
        )
        net.addRawNode(
            NetworkNode(
                "parent_b",
                "Parent B",
                "root_site",
                NodeType.site,
                800,
                800,
                networkJsonId="splynx:site:b",
            )
        )
        net.addRawNode(
            NetworkNode(
                "child_ap",
                "Child AP",
                "parent_a",
                NodeType.ap,
                300,
                300,
                networkJsonId="splynx:ap:child",
            )
        )

        net.prepareTree()
        editor = net.buildNativeTopologyEditorState("python/splynx")
        nodes = {node["node_id"]: node for node in editor["nodes"]}
        child = nodes["splynx:ap:child"]

        self.assertEqual(child["current_parent_node_id"], "splynx:site:a")
        self.assertTrue(child["can_move"])
        self.assertEqual(
            [parent["parent_node_id"] for parent in child["allowed_parents"]],
            ["splynx:site:a", "splynx:site:b"],
        )

        candidates = net.buildTopologyParentCandidates()
        candidate_by_id = {node["node_id"]: node for node in candidates["nodes"]}
        self.assertEqual(
            [candidate["node_id"] for candidate in candidate_by_id["splynx:ap:child"]["candidate_parents"]],
            ["splynx:site:a", "splynx:site:b"],
        )

    def test_native_topology_editor_exposes_root_peer_move_candidates(self):
        from integrationCommon import NetworkGraph, NetworkNode, NodeType

        net = NetworkGraph()
        net.addRawNode(
            NetworkNode(
                "gateway_a",
                "Gateway A",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:gateway-a",
            )
        )
        net.addRawNode(
            NetworkNode(
                "gateway_b",
                "Gateway B",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:gateway-b",
            )
        )
        net.addRawNode(
            NetworkNode(
                "gateway_c",
                "Gateway C",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="splynx:site:gateway-c",
            )
        )
        net.addRawNode(
            NetworkNode(
                "unattached",
                "LibreQoS Unattached [Site]",
                "",
                NodeType.site,
                1000,
                1000,
                networkJsonId="libreqos:generated:splynx:site:unattached",
            )
        )
        net.addRawNode(
            NetworkNode(
                "child_ap",
                "Child AP",
                "gateway_a",
                NodeType.ap,
                300,
                300,
                networkJsonId="splynx:ap:child",
            )
        )

        net.prepareTree()
        editor = net.buildNativeTopologyEditorState("python/splynx")
        nodes = {node["node_id"]: node for node in editor["nodes"]}
        child = nodes["splynx:ap:child"]

        self.assertEqual(child["current_parent_node_id"], "splynx:site:gateway-a")
        self.assertTrue(child["can_move"])
        self.assertEqual(
            [parent["parent_node_id"] for parent in child["allowed_parents"]],
            ["splynx:site:gateway-a", "splynx:site:gateway-b", "splynx:site:gateway-c"],
        )

        candidates = net.buildTopologyParentCandidates()
        candidate_by_id = {node["node_id"]: node for node in candidates["nodes"]}
        self.assertEqual(
            [candidate["node_id"] for candidate in candidate_by_id["splynx:ap:child"]["candidate_parents"]],
            ["splynx:site:gateway-a", "splynx:site:gateway-b", "splynx:site:gateway-c"],
        )

if __name__ == '__main__':
    unittest.main()
