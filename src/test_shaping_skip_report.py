import unittest

from shaping_skip_report import (
    build_unshaped_device_report,
    collect_parent_node_names,
    device_shaping_key,
    format_unshaped_device_line,
)


class TestShapingSkipReport(unittest.TestCase):
    def test_collect_parent_node_names_walks_nested_tree(self):
        network = {
            "Root": {
                "children": {
                    "AP_A": {
                        "children": {
                            "Sector_1": {}
                        }
                    }
                }
            }
        }

        self.assertEqual(
            collect_parent_node_names(network),
            {"Root", "AP_A", "Sector_1"},
        )

    def test_reports_unknown_parent_for_unattached_device(self):
        circuit = {
            "circuitID": "100",
            "circuitName": "Subscriber 100",
            "logicalParentNode": "Missing_AP",
            "ParentNode": "Missing_AP",
            "devices": [
                {"deviceID": "dev-1", "deviceName": "Radio A"},
            ],
        }

        skipped = build_unshaped_device_report(
            [circuit],
            shaped_device_keys=set(),
            valid_parent_nodes={"Root", "AP_A"},
            flat_network=False,
        )

        self.assertEqual(len(skipped), 1)
        self.assertEqual(skipped[0]["reasonCode"], "unknown_parent")
        self.assertIn("Missing_AP", skipped[0]["reasonText"])

    def test_reports_missing_parent_in_non_flat_network(self):
        circuit = {
            "circuitID": "101",
            "circuitName": "Subscriber 101",
            "logicalParentNode": "none",
            "ParentNode": "none",
            "devices": [
                {"deviceID": "dev-2", "deviceName": "Radio B"},
            ],
        }

        skipped = build_unshaped_device_report(
            [circuit],
            shaped_device_keys=set(),
            valid_parent_nodes={"Root", "Generated_PN_1"},
            flat_network=False,
        )

        self.assertEqual(len(skipped), 1)
        self.assertEqual(skipped[0]["reasonCode"], "missing_parent")

    def test_shaped_device_is_not_reported(self):
        circuit = {
            "circuitID": "102",
            "circuitName": "Subscriber 102",
            "logicalParentNode": "AP_A",
            "ParentNode": "AP_A",
            "devices": [
                {"deviceID": "dev-3", "deviceName": "Radio C"},
            ],
        }
        shaped_keys = {device_shaping_key(circuit, circuit["devices"][0])}

        skipped = build_unshaped_device_report(
            [circuit],
            shaped_device_keys=shaped_keys,
            valid_parent_nodes={"AP_A"},
            flat_network=False,
        )

        self.assertEqual(skipped, [])

    def test_duplicate_device_names_do_not_collide(self):
        shaped_circuit = {
            "circuitID": "200",
            "circuitName": "Subscriber 200",
            "logicalParentNode": "AP_A",
            "ParentNode": "AP_A",
            "devices": [
                {"deviceID": "dev-4", "deviceName": "Shared Name"},
            ],
        }
        skipped_circuit = {
            "circuitID": "201",
            "circuitName": "Subscriber 201",
            "logicalParentNode": "Ghost_AP",
            "ParentNode": "Ghost_AP",
            "devices": [
                {"deviceID": "dev-5", "deviceName": "Shared Name"},
            ],
        }

        skipped = build_unshaped_device_report(
            [shaped_circuit, skipped_circuit],
            shaped_device_keys={device_shaping_key(shaped_circuit, shaped_circuit["devices"][0])},
            valid_parent_nodes={"AP_A"},
            flat_network=False,
        )

        self.assertEqual(len(skipped), 1)
        self.assertEqual(skipped[0]["deviceID"], "dev-5")
        self.assertEqual(skipped[0]["reasonCode"], "unknown_parent")

    def test_flat_network_uses_flat_reason_when_device_is_unattached(self):
        circuit = {
            "circuitID": "300",
            "circuitName": "Subscriber 300",
            "logicalParentNode": "none",
            "ParentNode": "none",
            "devices": [
                {"deviceID": "dev-6", "deviceName": "Radio D"},
            ],
        }

        skipped = build_unshaped_device_report(
            [circuit],
            shaped_device_keys=set(),
            valid_parent_nodes={"Generated_PN_1"},
            flat_network=True,
        )

        self.assertEqual(len(skipped), 1)
        self.assertEqual(skipped[0]["reasonCode"], "unattached_flat_network")

    def test_format_unshaped_device_line_contains_context(self):
        line = format_unshaped_device_line(
            {
                "deviceID": "dev-7",
                "deviceName": "Radio E",
                "circuitID": "301",
                "circuitName": "Subscriber 301",
                "logicalParentNode": "Ghost_AP",
                "effectiveParentNode": "Ghost_AP",
                "reasonText": "ParentNode 'Ghost_AP' was not found in the shaping tree.",
            }
        )

        self.assertIn("DeviceID: dev-7", line)
        self.assertIn("CircuitName: Subscriber 301", line)
        self.assertIn("Reason: ParentNode 'Ghost_AP' was not found in the shaping tree.", line)


if __name__ == "__main__":
    unittest.main()
