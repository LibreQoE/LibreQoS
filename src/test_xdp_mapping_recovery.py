import unittest
import sys
import types


def install_libreqos_import_stubs():
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
    lqlib.get_libreqos_directory = lambda: "/tmp/libreqos"
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
    lqlib.plan_top_level_cpu_bins = lambda *_args, **_kwargs: {}
    lqlib.plan_class_identities = lambda *_args, **_kwargs: {}
    lqlib.fast_queues_fq_codel = lambda: False
    lqlib.shaping_cpu_count = lambda: 16
    lqlib.Bakery = DummyBakery
    sys.modules["liblqos_python"] = lqlib


install_libreqos_import_stubs()
sys.modules.pop("LibreQoS", None)

import LibreQoS


class FakeIpMapBatch:
    def __init__(self, submit_error=None):
        self.finished = False
        self.submitted = False
        self.submit_error = submit_error

    def finish_ip_mappings(self):
        self.finished = True

    def submit(self):
        self.submitted = True
        if self.submit_error is not None:
            raise self.submit_error


class XdpMappingRecoveryTests(unittest.TestCase):
    def test_ready_on_first_check_does_not_sleep(self):
        sleeps = []

        LibreQoS.wait_for_xdp_ip_mapping_ready(
            ready=lambda: True,
            sleep=lambda seconds: sleeps.append(seconds),
            now=lambda: 0.0,
        )

        self.assertEqual(sleeps, [])

    def test_readiness_after_one_false_check_sleeps_once(self):
        checks = iter([False, True])
        sleeps = []
        times = iter([0.0, 0.0, 0.0])

        LibreQoS.wait_for_xdp_ip_mapping_ready(
            ready=lambda: next(checks),
            sleep=lambda seconds: sleeps.append(seconds),
            now=lambda: next(times),
            timeout_seconds=5.0,
            interval_seconds=0.1,
        )

        self.assertEqual(sleeps, [0.1])

    def test_persistent_false_readiness_times_out(self):
        times = iter([0.0, 0.0, 0.2])

        with self.assertRaises(TimeoutError):
            LibreQoS.wait_for_xdp_ip_mapping_ready(
                ready=lambda: False,
                sleep=lambda _seconds: None,
                now=lambda: next(times),
                timeout_seconds=0.1,
                interval_seconds=0.1,
            )

    def test_readiness_os_error_reports_before_submit(self):
        batch = FakeIpMapBatch()
        reports = []

        def raise_os_error():
            raise OSError("Unable to inspect XDP IP mapping maps")

        LibreQoS.apply_xdp_ip_mappings(
            batch,
            {"queued_requests": 1},
            ready=raise_os_error,
            sleep=lambda _seconds: None,
            report_failure=lambda *args: reports.append(args),
            clear_recovered_issue=lambda *_args: True,
        )

        self.assertFalse(batch.finished)
        self.assertFalse(batch.submitted)
        self.assertEqual(reports[0][0], LibreQoS.XDP_IP_MAPPING_APPLY_FAILED)

    def test_submit_failure_reports_mapping_apply_failure(self):
        batch = FakeIpMapBatch(submit_error=OSError("Unable to open BPF map"))
        reports = []

        LibreQoS.apply_xdp_ip_mappings(
            batch,
            {"queued_requests": 1},
            ready=lambda: True,
            sleep=lambda _seconds: None,
            report_failure=lambda *args: reports.append(args),
            clear_recovered_issue=lambda *_args: True,
        )

        self.assertTrue(batch.finished)
        self.assertTrue(batch.submitted)
        self.assertEqual(reports[0][0], LibreQoS.XDP_IP_MAPPING_APPLY_FAILED)

    def test_success_clears_recovered_mapping_issue(self):
        batch = FakeIpMapBatch()
        cleared = []

        LibreQoS.apply_xdp_ip_mappings(
            batch,
            {"queued_requests": 1},
            ready=lambda: True,
            sleep=lambda _seconds: None,
            report_failure=lambda *_args: self.fail("unexpected failure report"),
            clear_recovered_issue=lambda code, dedupe_key: cleared.append((code, dedupe_key)) or True,
        )

        self.assertTrue(batch.finished)
        self.assertTrue(batch.submitted)
        self.assertEqual(
            cleared,
            [
                (
                    LibreQoS.XDP_IP_MAPPING_APPLY_FAILED,
                    LibreQoS.XDP_IP_MAPPING_APPLY_FAILED,
                )
            ],
        )


if __name__ == "__main__":
    unittest.main()
