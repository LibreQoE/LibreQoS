import unittest

from virtual_tree_nodes import (
    build_logical_to_physical_node_map,
    build_physical_network,
    is_virtual_node,
)

class TestVirtualTreeNodes(unittest.TestCase):
    def test_is_virtual_node(self):
        self.assertTrue(is_virtual_node({"virtual": True}))
        self.assertTrue(is_virtual_node({"type": "virtual"}))
        self.assertTrue(is_virtual_node({"type": "VIRTUAL"}))
        self.assertFalse(is_virtual_node({"virtual": False}))
        self.assertFalse(is_virtual_node({"type": "Site"}))
        self.assertFalse(is_virtual_node({}))

    def test_logical_to_physical_mapping_and_physical_promotion(self):
        logical = {
            "Region": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "Town": {
                        "downloadBandwidthMbps": 500,
                        "uploadBandwidthMbps": 500,
                        "virtual": True,
                        "children": {
                            "AP_A": {
                                "downloadBandwidthMbps": 200,
                                "uploadBandwidthMbps": 200,
                            }
                        },
                    },
                    "AP_B": {
                        "downloadBandwidthMbps": 300,
                        "uploadBandwidthMbps": 300,
                    },
                },
            }
        }

        mapping, virtual_nodes = build_logical_to_physical_node_map(logical)
        self.assertIn("Town", virtual_nodes)
        self.assertEqual(mapping["Town"], "Region")
        self.assertEqual(mapping["Region"], "Region")
        self.assertEqual(mapping["AP_A"], "AP_A")

        physical = build_physical_network(logical)
        self.assertIn("Region", physical)
        self.assertNotIn("Town", physical["Region"].get("children", {}))
        self.assertIn("AP_A", physical["Region"].get("children", {}))
        self.assertIn("AP_B", physical["Region"].get("children", {}))
        self.assertNotIn("virtual", physical["Region"])

    def test_promotion_collision_raises(self):
        logical = {
            "Region": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "AP_A": {"downloadBandwidthMbps": 300, "uploadBandwidthMbps": 300},
                    "Town": {
                        "downloadBandwidthMbps": 500,
                        "uploadBandwidthMbps": 500,
                        "virtual": True,
                        "children": {
                            "AP_A": {"downloadBandwidthMbps": 200, "uploadBandwidthMbps": 200}
                        },
                    },
                },
            }
        }

        with self.assertRaises(ValueError):
            build_physical_network(logical)


if __name__ == "__main__":
    unittest.main()
