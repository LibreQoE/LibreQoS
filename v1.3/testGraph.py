import unittest

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
        graph.reparentById()
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
        graph.reparentById()
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
        graph.reparentById()
        graph.promoteClientsWithChildren()
        self.assertEqual(graph.nodes[5].type, NodeType.clientWithChildren)
        self.assertEqual(graph.nodes[6].type, NodeType.client) # Test that a client is still a client

    def test_graph_render_to_pdf(self):
        """
        Requires that graphviz be installed with
        pip install graphviz
        And also the associated graphviz package for
        your platform.
        See: https://www.graphviz.org/download/
        Test that it creates a graphic
        """
        from integrationCommon import NetworkGraph, NetworkNode, NodeType
        graph = NetworkGraph()
        graph.addRawNode(NetworkNode("Site 1"))
        graph.addRawNode(NetworkNode("Site 2"))
        graph.addRawNode(NetworkNode("Client 1", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 2", parentId="Site 1", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 3", parentId="Site 2", type=NodeType.client))
        graph.addRawNode(NetworkNode("Client 4", parentId="Client 3", type=NodeType.client))
        graph.reparentById()
        graph.promoteClientsWithChildren()
        graph.plotNetworkGraph(True)
        from os.path import exists
        self.assertEqual(exists("network.pdf.pdf"), True)

if __name__ == '__main__':
    unittest.main()
