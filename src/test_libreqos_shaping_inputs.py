import csv
import importlib
import json
import os
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

_STUBBED_MODULES = ("pythonCheck", "liblqos_python", "deepdiff", "LibreQoS")
_ORIGINAL_MODULES = {name: sys.modules.get(name) for name in _STUBBED_MODULES}
LibreQoS = None


def install_libreqos_stubs():
    python_check = types.ModuleType("pythonCheck")
    python_check.checkPythonVersion = lambda: None
    sys.modules["pythonCheck"] = python_check

    deepdiff = types.ModuleType("deepdiff")
    deepdiff.DeepDiff = lambda *_args, **_kwargs: {}
    sys.modules["deepdiff"] = deepdiff

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
    lqlib.persist_topology_canonical_state = lambda _payload: None
    lqlib.validated_runtime_shaping_inputs_path = lambda: None
    lqlib.verify_published_shaping_generation = lambda: (True, "test-generation")
    lqlib.plan_top_level_cpu_bins = lambda *_args, **_kwargs: {}
    lqlib.plan_class_identities = lambda *_args, **_kwargs: {}
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
    def test_refresh_shapers_retires_legacy_file_chain_in_integration_mode(self):
        # Phase C S4: the topology runtime/bridge is ALWAYS the shaper; the
        # legacy python file chain retired.
        with patch.object(LibreQoS, "validateNetworkAndDevices") as validate:
            result = LibreQoS.refreshShapers()
        self.assertTrue(result)
        validate.assert_not_called()


if __name__ == "__main__":
    unittest.main()
