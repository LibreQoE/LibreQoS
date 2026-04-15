import csv
import importlib
import json
import os
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

_STUBBED_MODULES = ("pythonCheck", "liblqos_python", "LibreQoS")
_ORIGINAL_MODULES = {name: sys.modules.get(name) for name in _STUBBED_MODULES}
LibreQoS = None


def install_libreqos_stubs():
    python_check = types.ModuleType("pythonCheck")
    python_check.checkPythonVersion = lambda: None
    sys.modules["pythonCheck"] = python_check

    lqlib = types.ModuleType("liblqos_python")
    lqlib.is_lqosd_alive = lambda: True
    lqlib.clear_ip_mappings = lambda: None
    lqlib.delete_ip_mapping = lambda *_args, **_kwargs: None
    lqlib.validate_shaped_devices = lambda: "OK"
    lqlib.is_libre_already_running = lambda: False
    lqlib.create_lock_file = lambda: None
    lqlib.free_lock_file = lambda: None
    lqlib.add_ip_mapping = lambda *_args, **_kwargs: None

    class DummyBatchedCommands:
        pass

    class DummyBakery:
        pass

    lqlib.BatchedCommands = DummyBatchedCommands
    lqlib.check_config = lambda: None
    lqlib.sqm = lambda: "cake"
    lqlib.upstream_bandwidth_capacity_download_mbps = lambda: 1000
    lqlib.upstream_bandwidth_capacity_upload_mbps = lambda: 1000
    lqlib.interface_a = lambda: "eth0"
    lqlib.interface_b = lambda: "eth1"
    lqlib.enable_actual_shell_commands = lambda: False
    lqlib.use_bin_packing_to_balance_cpu = lambda: False
    lqlib.queue_mode = lambda: "shape"
    lqlib.run_shell_commands_as_sudo = lambda: False
    lqlib.generated_pn_download_mbps = lambda: 1000
    lqlib.generated_pn_upload_mbps = lambda: 1000
    lqlib.queues_available_override = lambda: 0
    lqlib.on_a_stick = lambda: False
    lqlib.get_tree_weights = lambda: {}
    lqlib.get_weights = lambda: {}
    lqlib.is_network_flat = lambda: False
    lqlib.get_libreqos_directory = lambda: "/tmp/libreqos"  # nosec B108
    lqlib.enable_insight_topology = lambda: False
    lqlib.is_insight_enabled = lambda: False
    lqlib.scheduler_error = lambda *_args, **_kwargs: None
    lqlib.xdp_ip_mapping_capacity = lambda: 1024
    lqlib.overrides_circuit_adjustments_effective = lambda: []
    lqlib.automatic_import_uisp = lambda: False
    lqlib.automatic_import_splynx = lambda: False
    lqlib.automatic_import_powercode = lambda: False
    lqlib.automatic_import_sonar = lambda: False
    lqlib.automatic_import_wispgate = lambda: False
    lqlib.automatic_import_netzur = lambda: False
    lqlib.automatic_import_visp = lambda: False
    lqlib.topology_import_ingress_enabled = lambda: False
    lqlib.calculate_topology_source_generation = lambda: "test-generation"
    lqlib.plan_top_level_cpu_bins = lambda *_args, **_kwargs: {}
    lqlib.plan_class_identities = lambda *_args, **_kwargs: {}
    lqlib.fast_queues_fq_codel = lambda: False
    lqlib.shaping_cpu_count = lambda: 16
    lqlib.Bakery = DummyBakery
    sys.modules["liblqos_python"] = lqlib

def setUpModule():
    global LibreQoS
    for name in _STUBBED_MODULES:
        sys.modules.pop(name, None)
    install_libreqos_stubs()
    LibreQoS = importlib.import_module("LibreQoS")


def tearDownModule():
    for name, module in _ORIGINAL_MODULES.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


class TestLibreQoSShapingInputs(unittest.TestCase):
    def test_attachment_lookup_candidates_preserve_generated_parent_name_without_node_id(self):
        candidates = LibreQoS._attachment_lookup_candidates("Generated_PN_1", {})
        self.assertEqual(candidates, ["Generated_PN_1"])

    def test_shaping_inputs_freshness_tracks_circuit_anchors(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            shaping_inputs = os.path.join(temp_dir, "shaping_inputs.json")
            shaped_devices = os.path.join(temp_dir, "ShapedDevices.csv")
            network_json = os.path.join(temp_dir, "network.json")
            circuit_anchors = os.path.join(temp_dir, "circuit_anchors.json")

            for path in (shaping_inputs, shaped_devices, network_json, circuit_anchors):
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("{}\n")

            now = time.time()
            os.utime(shaped_devices, (now - 20, now - 20))
            os.utime(network_json, (now - 20, now - 20))
            os.utime(shaping_inputs, (now - 10, now - 10))
            os.utime(circuit_anchors, (now - 5, now - 5))

            self.assertFalse(
                LibreQoS._shaping_inputs_are_fresh(
                    shaping_inputs, shaped_devices, network_json, circuit_anchors
                )
            )

    def test_shaping_inputs_freshness_accepts_ready_topology_runtime_status(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            shaping_inputs = os.path.join(temp_dir, "shaping_inputs.json")
            shaped_devices = os.path.join(temp_dir, "ShapedDevices.csv")
            network_json = os.path.join(temp_dir, "network.effective.json")
            topology_state = os.path.join(temp_dir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            status_path = os.path.join(topology_state, "topology_runtime_status.json")

            for path in (shaping_inputs, shaped_devices, network_json):
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("{}\n")

            now = time.time()
            os.utime(shaped_devices, (now - 20, now - 20))
            os.utime(shaping_inputs, (now - 10, now - 10))
            os.utime(network_json, (now - 5, now - 5))

            with open(status_path, "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "test-generation",
                        "shaping_generation": "shape-1",
                        "ready": True,
                        "shaping_inputs_path": shaping_inputs,
                    },
                    handle,
                )

            with patch.object(LibreQoS, "get_libreqos_directory", return_value=temp_dir):
                self.assertTrue(
                    LibreQoS._shaping_inputs_are_fresh(
                        shaping_inputs, shaped_devices, network_json
                    )
                )

    def test_shaping_inputs_freshness_rejects_stale_topology_runtime_status_generation(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            shaping_inputs = os.path.join(temp_dir, "shaping_inputs.json")
            shaped_devices = os.path.join(temp_dir, "ShapedDevices.csv")
            network_json = os.path.join(temp_dir, "network.effective.json")
            topology_state = os.path.join(temp_dir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            status_path = os.path.join(topology_state, "topology_runtime_status.json")

            for path in (shaping_inputs, shaped_devices, network_json):
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("{}\n")

            now = time.time()
            os.utime(shaped_devices, (now - 20, now - 20))
            os.utime(shaping_inputs, (now - 10, now - 10))
            os.utime(network_json, (now - 5, now - 5))

            with open(status_path, "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "old-generation",
                        "shaping_generation": "shape-1",
                        "ready": True,
                        "shaping_inputs_path": shaping_inputs,
                    },
                    handle,
                )

            with patch.object(LibreQoS, "get_libreqos_directory", return_value=temp_dir):
                self.assertFalse(
                    LibreQoS._shaping_inputs_are_fresh(
                        shaping_inputs, shaped_devices, network_json
                    )
                )

    def test_shaping_inputs_freshness_rejects_ready_status_without_shaping_generation(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            shaping_inputs = os.path.join(temp_dir, "shaping_inputs.json")
            shaped_devices = os.path.join(temp_dir, "ShapedDevices.csv")
            network_json = os.path.join(temp_dir, "network.effective.json")
            topology_state = os.path.join(temp_dir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            status_path = os.path.join(topology_state, "topology_runtime_status.json")

            for path in (shaping_inputs, shaped_devices, network_json):
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("{}\n")

            with open(status_path, "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "test-generation",
                        "ready": True,
                        "shaping_inputs_path": shaping_inputs,
                    },
                    handle,
                )

            with patch.object(LibreQoS, "get_libreqos_directory", return_value=temp_dir):
                self.assertFalse(
                    LibreQoS._shaping_inputs_are_fresh(
                        shaping_inputs, shaped_devices, network_json
                    )
                )

    def test_load_subscriber_circuits_accepts_diy_id_alias(self):
        header = [
            "Circuit ID",
            "Circuit Name",
            "Device ID",
            "Device Name",
            "Parent Node",
            "Parent Node ID",
            "id",
            "MAC",
            "IPv4",
            "IPv6",
            "Download Min Mbps",
            "Upload Min Mbps",
            "Download Max Mbps",
            "Upload Max Mbps",
            "Comment",
        ]
        row = [
            "100",
            "Subscriber 100",
            "device-100",
            "Radio 100",
            "Tower-A",
            "uisp:device:tower-a",
            "uisp:site:site-100",
            "aa:bb:cc:dd:ee:ff",
            "100.64.0.10/32",
            "",
            "10",
            "10",
            "100",
            "100",
            "DIY id alias",
        ]

        with tempfile.NamedTemporaryFile("w", encoding="utf-8", newline="", delete=False) as handle:
            writer = csv.writer(handle)
            writer.writerow(header)
            writer.writerow(row)
            path = handle.name

        try:
            circuits, _ = LibreQoS.loadSubscriberCircuits(path)
        finally:
            os.remove(path)

        self.assertEqual(len(circuits), 1)
        self.assertEqual(circuits[0]["AnchorNodeID"], "uisp:site:site-100")

    def test_load_subscriber_circuits_for_shaping_requires_runtime_artifacts_for_diy(self):
        with patch.object(LibreQoS, "_shaping_inputs_are_fresh", return_value=False):
            with self.assertRaises(LibreQoS.RefreshFailure):
                LibreQoS.loadSubscriberCircuitsForShaping(
                    "/tmp/ShapedDevices.csv",  # nosec B108
                    "/tmp/network.json",  # nosec B108
                )

    def test_load_subscriber_circuits_for_shaping_requires_valid_runtime_payload(self):
        with patch.object(LibreQoS, "_shaping_inputs_are_fresh", return_value=True):
            with patch.object(
                LibreQoS,
                "loadSubscriberCircuitsFromShapingInputs",
                side_effect=ValueError("bad payload"),
            ):
                with self.assertRaises(LibreQoS.RefreshFailure):
                    LibreQoS.loadSubscriberCircuitsForShaping(
                        "/tmp/ShapedDevices.csv",  # nosec B108
                        "/tmp/network.json",  # nosec B108
                    )


if __name__ == "__main__":
    unittest.main()
