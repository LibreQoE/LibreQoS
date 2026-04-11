import importlib
import json
import os
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch


def install_scheduler_stubs():
    libre = types.ModuleType("LibreQoS")
    libre.refreshShapers = Mock()
    libre.refreshShapersUpdateOnly = Mock()
    sys.modules["LibreQoS"] = libre

    lqlib = types.ModuleType("liblqos_python")
    lqlib.automatic_import_uisp = lambda: False
    lqlib.automatic_import_splynx = lambda: False
    lqlib.queue_refresh_interval_mins = lambda: 30
    lqlib.automatic_import_powercode = lambda: False
    lqlib.automatic_import_sonar = lambda: False
    lqlib.influx_db_enabled = lambda: False
    # Test-only fake install root.
    lqlib.get_libreqos_directory = lambda: "/tmp/libreqos"  # nosec B108
    lqlib.blackboard_finish = Mock()
    lqlib.blackboard_submit = Mock()
    lqlib.automatic_import_wispgate = lambda: False
    lqlib.enable_insight_topology = lambda: False
    lqlib.insight_topology_role = lambda: "primary"
    lqlib.automatic_import_netzur = lambda: False
    lqlib.automatic_import_visp = lambda: False
    lqlib.calculate_hash = lambda: 0
    lqlib.calculate_shaping_runtime_hash = lambda: 0
    lqlib.calculate_topology_source_generation = lambda: "test-generation"
    lqlib.topology_import_ingress_enabled = lambda: False
    lqlib.efficiency_core_ids = lambda: []
    lqlib.scheduler_alive = Mock()
    lqlib.scheduler_error = Mock()
    lqlib.scheduler_output = Mock()
    lqlib.scheduler_progress = Mock()
    lqlib.wait_for_bus_ready = Mock(return_value=True)
    lqlib.overrides_persistent_devices_effective = lambda: []
    lqlib.overrides_persistent_devices_materialized = lambda: []
    lqlib.overrides_circuit_adjustments_effective = lambda: []
    lqlib.overrides_circuit_adjustments_materialized = lambda: []
    lqlib.overrides_network_adjustments_effective = lambda: []
    lqlib.overrides_network_adjustments_materialized = lambda: []
    sys.modules["liblqos_python"] = lqlib

    apscheduler_pkg = types.ModuleType("apscheduler")
    sys.modules["apscheduler"] = apscheduler_pkg
    apscheduler_schedulers = types.ModuleType("apscheduler.schedulers")
    sys.modules["apscheduler.schedulers"] = apscheduler_schedulers
    apscheduler_background = types.ModuleType("apscheduler.schedulers.background")
    sys.modules["apscheduler.schedulers.background"] = apscheduler_background
    apscheduler_executors = types.ModuleType("apscheduler.executors")
    sys.modules["apscheduler.executors"] = apscheduler_executors
    apscheduler_pool = types.ModuleType("apscheduler.executors.pool")
    sys.modules["apscheduler.executors.pool"] = apscheduler_pool

    class FakeBlockingScheduler:
        def __init__(self, *args, **kwargs):
            self.args = args
            self.kwargs = kwargs

        def add_job(self, *args, **kwargs):
            return None

        def start(self):
            return None

    class FakeThreadPoolExecutor:
        def __init__(self, *args, **kwargs):
            self.args = args
            self.kwargs = kwargs

    apscheduler_background.BlockingScheduler = FakeBlockingScheduler
    apscheduler_pool.ThreadPoolExecutor = FakeThreadPoolExecutor


install_scheduler_stubs()
scheduler = importlib.import_module("scheduler")


class TestSchedulerAffinity(unittest.TestCase):
    def test_run_integration_subprocess_uses_efficiency_core_affinity(self):
        result = types.SimpleNamespace(returncode=0, stdout="", stderr="")

        def fake_run(cmd, **kwargs):
            self.assertEqual(cmd, ["fake-binary"])
            self.assertIn("preexec_fn", kwargs)
            kwargs["preexec_fn"]()
            return result

        with patch.object(scheduler, "efficiency_core_ids", return_value=[11, 10, 10]):
            with patch.object(scheduler.os, "sched_setaffinity") as mock_affinity:
                with patch.object(scheduler.subprocess, "run", side_effect=fake_run):
                    observed = scheduler.run_integration_subprocess(
                        ["fake-binary"],
                        label="fake integration",
                    )

        self.assertIs(observed, result)
        mock_affinity.assert_called_once_with(0, {10, 11})

    def test_run_integration_subprocess_retries_without_affinity_on_failure(self):
        result = types.SimpleNamespace(returncode=0, stdout="", stderr="")
        calls = []

        def fake_run(cmd, **kwargs):
            calls.append(kwargs.copy())
            if "preexec_fn" in kwargs:
                raise RuntimeError("preexec failed")
            return result

        with patch.object(scheduler, "efficiency_core_ids", return_value=[10]):
            with patch.object(scheduler.subprocess, "run", side_effect=fake_run):
                with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                    with patch("builtins.print"):
                        observed = scheduler.run_integration_subprocess(
                            ["fake-binary"],
                            label="fake integration",
                        )

        self.assertIs(observed, result)
        self.assertEqual(len(calls), 2)
        self.assertIn("preexec_fn", calls[0])
        self.assertNotIn("preexec_fn", calls[1])
        mock_scheduler_error.assert_called_once()

    def test_run_integration_subprocess_skips_affinity_without_efficiency_cores(self):
        result = types.SimpleNamespace(returncode=0, stdout="", stderr="")

        def fake_run(cmd, **kwargs):
            self.assertNotIn("preexec_fn", kwargs)
            return result

        with patch.object(scheduler, "efficiency_core_ids", return_value=[]):
            with patch.object(scheduler.subprocess, "run", side_effect=fake_run):
                observed = scheduler.run_integration_subprocess(
                    ["fake-binary"],
                    label="fake integration",
                )

        self.assertIs(observed, result)

    def test_post_integration_hook_remains_unpinned(self):
        result = types.SimpleNamespace(returncode=0, stdout="", stderr="")

        with patch.object(scheduler, "automatic_import_uisp", return_value=True):
            # Test-only fake install root.
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):  # nosec B108
                with patch.object(scheduler, "run_integration_subprocess", return_value=result) as mock_run:
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=True):
                            with patch.object(scheduler.subprocess, "Popen") as mock_popen:
                                scheduler.importFromCRM()

        mock_run.assert_called_once()
        mock_popen.assert_called_once_with(
            "/tmp/libreqos/bin/post_integration_hook.sh",  # nosec B108
            cwd="/tmp/libreqos/bin",  # nosec B108
        )


class TestSchedulerErrorReporting(unittest.TestCase):
    def setUp(self):
        scheduler.set_scheduler_status_bus_enabled(True)

    def test_python_integration_output_does_not_set_scheduler_error(self):
        result = types.SimpleNamespace(returncode=0, stdout="normal info\n", stderr="")

        with patch.object(scheduler, "run_integration_subprocess", return_value=result):
            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                    with patch("builtins.print"):
                        scheduler.run_python_integration(
                            "integrationExample",
                            "importExample",
                            label="Example",
                        )

        mock_scheduler_error.assert_not_called()
        mock_scheduler_output.assert_called_once()
        self.assertIn(
            "Example completed successfully. Captured 1 line(s) of output.",
            mock_scheduler_output.call_args.args[0],
        )

    def test_python_integration_nonzero_exit_sets_scheduler_error(self):
        result = types.SimpleNamespace(returncode=2, stdout="normal info\n", stderr="")

        with patch.object(scheduler, "run_integration_subprocess", return_value=result):
            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                    with patch("builtins.print"):
                        scheduler.run_python_integration(
                            "integrationExample",
                            "importExample",
                            label="Example",
                        )

        mock_scheduler_error.assert_called_once()
        self.assertIn(
            "Example exited with code 2. Continuing.",
            mock_scheduler_error.call_args.args[0],
        )
        self.assertIn("Output preview:\nnormal info", mock_scheduler_error.call_args.args[0])
        self.assertIn(
            "Full output saved to /tmp/lqos_scheduler_example_",
            mock_scheduler_error.call_args.args[0],
        )
        mock_scheduler_output.assert_not_called()

    def test_import_from_crm_clears_error_and_keeps_success_output_non_error(self):
        result = types.SimpleNamespace(returncode=0, stdout="uisp info\n", stderr="")

        with patch.object(scheduler, "automatic_import_uisp", return_value=True):
            # Test-only fake install root.
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):  # nosec B108
                with patch.object(scheduler, "run_integration_subprocess", return_value=result):
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=False):
                            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                    with patch("builtins.print"):
                                        scheduler.importFromCRM()

        self.assertEqual(mock_scheduler_error.call_args_list, [(( "",),)])
        self.assertEqual(mock_scheduler_output.call_args_list[0], (("",),))
        self.assertIn(
            "UISP integration completed successfully. Captured 1 line(s) of output.",
            mock_scheduler_output.call_args_list[1].args[0],
        )

    def test_import_from_crm_reports_nonzero_exit(self):
        result = types.SimpleNamespace(returncode=1, stdout="uisp info\n", stderr="")

        with patch.object(scheduler, "automatic_import_uisp", return_value=True):
            # Test-only fake install root.
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):  # nosec B108
                with patch.object(scheduler, "run_integration_subprocess", return_value=result):
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=False):
                            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                    with patch("builtins.print"):
                                        scheduler.importFromCRM()

        self.assertEqual(mock_scheduler_error.call_args_list[0], (("",),))
        self.assertIn(
            "UISP integration exited with code 1. Continuing.",
            mock_scheduler_error.call_args_list[1].args[0],
        )
        self.assertIn(
            "Output preview:\nuisp info",
            mock_scheduler_error.call_args_list[1].args[0],
        )
        self.assertIn(
            "Full output saved to /tmp/lqos_scheduler_uisp_integration_",
            mock_scheduler_error.call_args_list[1].args[0],
        )
        self.assertEqual(mock_scheduler_output.call_args_list, [(( "",),)])

    def test_run_scheduler_main_stays_alive_on_startup_refresh_failure(self):
        fake_ads = Mock()

        with patch.object(scheduler, "ads", fake_ads):
            with patch.object(scheduler, "ensure_bus_ready"):
                with patch.object(
                    scheduler,
                    "importAndShapeFullReload",
                    side_effect=RuntimeError("runtime contract failed"),
                ):
                    with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                        with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                            with patch.object(scheduler.atexit, "register"):
                                with patch.object(scheduler, "not_dead_yet"):
                                    with patch("traceback.print_exc"):
                                        with patch("builtins.print"):
                                            scheduler.run_scheduler_main()

        self.assertEqual(scheduler.shaping_runtime_hash, 0)
        mock_scheduler_error.assert_called_once_with(
            "Scheduler startup shaping refresh failed: runtime contract failed"
        )
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "degraded",
                    "Scheduler running with topology/runtime error",
                )
                for call in mock_progress.call_args_list
            )
        )
        self.assertEqual(fake_ads.add_job.call_count, 3)
        fake_ads.start.assert_called_once()

    def test_topology_runtime_refresh_tick_reports_refresh_failure(self):
        with patch.object(scheduler, "ensure_topology_runtime_process"):
            with patch.object(
                scheduler,
                "topology_runtime_readiness_detail",
                return_value=(True, "", "generation-1"),
            ):
                with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=5):
                    with patch.object(scheduler, "refreshShapers", side_effect=RuntimeError("bad runtime")):
                        with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                            with patch("builtins.print"):
                                scheduler.shaping_runtime_hash = 1
                                scheduler.topology_runtime_refresh_tick()

        mock_scheduler_error.assert_called_once_with(
            "Topology runtime refresh failed: bad runtime"
        )

    def test_topology_runtime_refresh_tick_skips_until_initial_shaping_succeeds(self):
        with patch.object(scheduler, "ensure_topology_runtime_process") as mock_ensure:
            with patch.object(scheduler, "calculate_shaping_runtime_hash") as mock_hash:
                scheduler.shaping_runtime_hash = 0
                scheduler.topology_runtime_refresh_tick()

        mock_ensure.assert_not_called()
        mock_hash.assert_not_called()

    def test_import_and_shape_full_reload_reenables_status_bus_after_success(self):
        scheduler.set_scheduler_status_bus_enabled(False)

        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process"):
                with patch.object(scheduler, "publish_scheduler_progress"):
                    with patch.object(scheduler, "enable_insight_topology", return_value=False):
                        with patch.object(scheduler, "refreshShapers"):
                            with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=9):
                                scheduler.importAndShapeFullReload()

        self.assertTrue(scheduler.scheduler_status_bus_enabled)


class TestTopologyRuntimeReadiness(unittest.TestCase):
    def test_missing_status_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("still building outputs", detail)

    def test_stale_status_generation_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            with open(os.path.join(tempdir, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-old",
                        "ready": True,
                    },
                    handle,
                )
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-new",
                ):
                    ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-new")
        self.assertIn("still building outputs", detail)

    def test_ready_false_status_blocks_current_generation(self):
        with tempfile.TemporaryDirectory() as tempdir:
            with open(os.path.join(tempdir, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "ready": False,
                        "error": "Unable to publish shaping inputs",
                    },
                    handle,
                )
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("failed for the current source generation", detail)
        self.assertIn("Unable to publish shaping inputs", detail)

    def test_ready_true_matching_status_allows_current_generation(self):
        with tempfile.TemporaryDirectory() as tempdir:
            with open(os.path.join(tempdir, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "ready": True,
                        "error": None,
                    },
                    handle,
                )
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertTrue(ready)
        self.assertEqual(detail, "")
        self.assertEqual(generation, "generation-1")


class TestTopologyRuntimeGating(unittest.TestCase):
    def test_full_reload_skips_refresh_when_topology_runtime_not_ready(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(scheduler, "report_topology_runtime_not_ready") as mock_report:
                    with patch.object(scheduler, "refreshShapers") as mock_refresh:
                        with patch.object(scheduler, "publish_scheduler_progress"):
                            scheduler.importAndShapeFullReload()

        mock_refresh.assert_not_called()
        mock_report.assert_called_once()

    def test_partial_reload_skips_refresh_when_topology_runtime_not_ready(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(scheduler, "report_topology_runtime_not_ready") as mock_report:
                    with patch.object(scheduler, "refreshShapers") as mock_refresh:
                        with patch.object(scheduler, "publish_scheduler_progress"):
                            scheduler.importAndShapePartialReload()

        mock_refresh.assert_not_called()
        mock_report.assert_called_once()


class TestSchedulerOverrideMerge(unittest.TestCase):
    def test_merge_rows_replaces_matching_device_id(self):
        existing = [["93", "Name", "splynx_service_93", "Name", "AP", "MAC", "1.1.1.1", "", "1", "1", "330", "330", "", ""]]
        override = [["93", "Name", "splynx_service_93", "Name", "AP", "MAC", "1.1.1.1/32", "", "1", "1", "330", "330", "", "fq_codel/fq_codel"]]

        merged, changed = scheduler.merge_rows_replace_by_device_id(existing, override)

        self.assertTrue(changed)
        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0][2], "splynx_service_93")
        self.assertEqual(merged[0][6], "1.1.1.1/32")
        self.assertEqual(merged[0][13], "fq_codel/fq_codel")

    def test_merge_rows_appends_unmatched_non_splynx_override(self):
        existing = [["93", "Name", "splynx_service_93", "Name", "AP", "MAC", "1.1.1.1", "", "1", "1", "330", "330", "", ""]]
        override = [["145", "Other", "legacy_device_1", "Other", "AP", "MAC2", "2.2.2.2", "", "1", "1", "300", "300", "", ""]]

        merged, changed = scheduler.merge_rows_replace_by_device_id(existing, override)

        self.assertTrue(changed)
        self.assertEqual(len(merged), 2)
        self.assertEqual(merged[1][2], "legacy_device_1")

    def test_apply_lqos_overrides_device_adjust_sqm_only_updates_sqm_column(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node", "MAC",
            "IPv4", "IPv6", "Download Min Mbps", "Upload Min Mbps", "Download Max Mbps",
            "Upload Max Mbps", "Comment", "SQM"
        ]
        rows = [[
            "93", "Name", "splynx_service_93", "Name", "AP", "MAC", "1.1.1.1", "",
            "1", "1", "330", "330", "", ""
        ]]

        # Test-only fake csv path.
        with patch.object(scheduler, "shaped_devices_csv_path", return_value="/tmp/ShapedDevices.csv"):  # nosec B108
            with patch.object(scheduler, "read_shaped_devices_csv", return_value=(header, rows)):
                with patch.object(scheduler, "overrides_persistent_devices_materialized", return_value=[]):
                    with patch.object(
                        scheduler,
                        "overrides_circuit_adjustments_materialized",
                        return_value=[{
                            "type": "device_adjust_sqm",
                            "device_id": "splynx_service_93",
                            "sqm_override": "fq_codel/fq_codel",
                        }],
                    ):
                        with patch.object(scheduler, "write_shaped_devices_csv") as mock_write:
                            scheduler.apply_lqos_overrides()

        written_rows = mock_write.call_args.args[2]
        self.assertEqual(written_rows[0][10], "330")
        self.assertEqual(written_rows[0][11], "330")
        self.assertEqual(written_rows[0][13], "fq_codel/fq_codel")

    def test_apply_lqos_overrides_reparent_clears_parent_node_id_when_present(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node",
            "Parent Node ID", "MAC", "IPv4", "IPv6", "Download Min Mbps",
            "Upload Min Mbps", "Download Max Mbps", "Upload Max Mbps", "Comment",
        ]
        rows = [[
            "93", "Name", "splynx_service_93", "Name", "AP",
            "uisp:device:ap-1", "MAC", "1.1.1.1", "",
            "1", "1", "330", "330", "",
        ]]

        with patch.object(scheduler, "shaped_devices_csv_path", return_value="/tmp/ShapedDevices.csv"):  # nosec B108
            with patch.object(scheduler, "read_shaped_devices_csv", return_value=(header, rows)):
                with patch.object(scheduler, "overrides_persistent_devices_materialized", return_value=[]):
                    with patch.object(
                        scheduler,
                        "overrides_circuit_adjustments_materialized",
                        return_value=[{
                            "type": "reparent_circuit",
                            "circuit_id": "93",
                            "parent_node": "AP-Updated",
                        }],
                    ):
                        with patch.object(scheduler, "write_shaped_devices_csv") as mock_write:
                            scheduler.apply_lqos_overrides()

        written_rows = mock_write.call_args.args[2]
        self.assertEqual(written_rows[0][4], "AP-Updated")
        self.assertEqual(written_rows[0][5], "")

    def test_apply_lqos_overrides_updates_canonical_only_for_integration_ingress(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node", "MAC",
            "IPv4", "IPv6", "Download Min Mbps", "Upload Min Mbps", "Download Max Mbps",
            "Upload Max Mbps", "Comment",
        ]
        rows = [[
            "93", "Name", "splynx_service_93", "Name", "AP", "MAC", "1.1.1.1", "",
            "1", "1", "330", "330", "",
        ]]
        canonical_state = {
            "compatibility_network_json": {
                "NodeB": {
                    "downloadBandwidthMbps": 200,
                    "uploadBandwidthMbps": 100,
                    "children": {},
                }
            },
            "nodes": [
                {
                    "node_id": "node-b",
                    "node_name": "NodeB",
                    "rate_input": {
                        "intrinsic_download_mbps": 200,
                        "intrinsic_upload_mbps": 100,
                    },
                }
            ],
        }

        with patch.object(scheduler, "shaped_devices_csv_path", return_value="/tmp/ShapedDevices.csv"):  # nosec B108
            with patch.object(scheduler, "read_shaped_devices_csv", return_value=(header, rows)):
                with patch.object(scheduler, "overrides_persistent_devices_materialized", return_value=[]):
                    with patch.object(scheduler, "overrides_circuit_adjustments_materialized", return_value=[]):
                        with patch.object(
                            scheduler,
                            "overrides_network_adjustments_materialized",
                            return_value=[{
                                "type": "adjust_site_speed",
                                "node_id": "node-b",
                                "site_name": "NodeB",
                                "download_bandwidth_mbps": 80,
                                "upload_bandwidth_mbps": 40,
                            }],
                        ):
                            with patch.object(scheduler, "topology_import_ingress_enabled", return_value=True):
                                with patch.object(scheduler, "load_topology_canonical_state", return_value=canonical_state):
                                    with patch.object(scheduler, "write_topology_canonical_state") as mock_write_canonical:
                                        with patch.object(scheduler, "load_network_json") as mock_load_network:
                                            with patch.object(scheduler, "write_network_json") as mock_write_network:
                                                with patch.object(scheduler, "write_shaped_devices_csv") as mock_write_sd:
                                                    scheduler.apply_lqos_overrides()

        mock_load_network.assert_not_called()
        mock_write_network.assert_not_called()
        mock_write_sd.assert_not_called()
        mock_write_canonical.assert_called_once()

    def test_override_devices_to_rows_preserves_anchor_node_id(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node",
            "Parent Node ID", "Anchor Node ID", "MAC", "IPv4", "IPv6",
            "Download Min Mbps", "Upload Min Mbps", "Download Max Mbps", "Upload Max Mbps",
            "Comment", "SQM",
        ]
        rows = scheduler.override_devices_to_rows(
            [{
                "circuitID": "93",
                "circuitName": "Name",
                "deviceID": "device-93",
                "deviceName": "Name",
                "ParentNode": "AP",
                "ParentNodeID": "uisp:device:ap-1",
                "AnchorNodeID": "uisp:site:site-93",
                "mac": "MAC",
                "ipv4s": ["1.1.1.1"],
                "ipv6s": [],
                "minDownload": 1,
                "minUpload": 1,
                "maxDownload": 330,
                "maxUpload": 330,
                "comment": "",
                "sqm": "fq_codel/fq_codel",
            }],
            header,
            include_sqm=True,
        )

        self.assertEqual(rows[0][6], "uisp:site:site-93")
        self.assertEqual(rows[0][15], "fq_codel/fq_codel")

    def test_override_devices_to_rows_preserves_diy_id_header_alias(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node",
            "Parent Node ID", "id", "MAC", "IPv4", "IPv6",
            "Download Min Mbps", "Upload Min Mbps", "Download Max Mbps", "Upload Max Mbps",
            "Comment",
        ]
        rows = scheduler.override_devices_to_rows(
            [{
                "circuitID": "93",
                "circuitName": "Name",
                "deviceID": "device-93",
                "deviceName": "Name",
                "ParentNode": "AP",
                "ParentNodeID": "uisp:device:ap-1",
                "AnchorNodeID": "uisp:site:site-93",
                "mac": "MAC",
                "ipv4s": ["1.1.1.1"],
                "ipv6s": [],
                "minDownload": 1,
                "minUpload": 1,
                "maxDownload": 330,
                "maxUpload": 330,
                "comment": "",
            }],
            header,
            include_sqm=False,
        )

        self.assertEqual(rows[0][6], "uisp:site:site-93")

    def test_topology_runtime_output_paths_include_shaping_inputs(self):
        # Test-only fake install root.
        with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):  # nosec B108
            paths = scheduler.topology_runtime_output_paths()

        self.assertIn("/tmp/libreqos/shaping_inputs.json", paths)  # nosec B108

    def test_apply_network_adjustments_uses_materialized_adjustments(self):
        network = {
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "SiteA": {
                        "downloadBandwidthMbps": 100,
                        "uploadBandwidthMbps": 50,
                        "children": {},
                    },
                    "NodeB": {
                        "id": "node-b",
                        "downloadBandwidthMbps": 200,
                        "uploadBandwidthMbps": 100,
                        "virtual": False,
                        "children": {},
                    },
                },
            }
        }

        with patch.object(
            scheduler,
            "overrides_network_adjustments_materialized",
            return_value=[
                {
                    "type": "adjust_site_speed",
                    "node_id": "node-b",
                    "site_name": "NodeB",
                    "download_bandwidth_mbps": 80.5,
                    "upload_bandwidth_mbps": 40.25,
                },
                {
                    "type": "set_node_virtual",
                    "node_name": "NodeB",
                    "virtual": True,
                },
            ],
        ):
            changed = scheduler.apply_network_adjustments(network)

        self.assertTrue(changed)
        node = network["Root"]["children"]["NodeB"]
        self.assertEqual(node["downloadBandwidthMbps"], 80.5)
        self.assertEqual(node["uploadBandwidthMbps"], 40.25)
        self.assertTrue(network["Root"]["children"]["NodeB"]["virtual"])

    def test_apply_network_adjustments_keeps_legacy_name_based_matching(self):
        network = {
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "SiteA": {
                        "downloadBandwidthMbps": 100,
                        "uploadBandwidthMbps": 50,
                        "children": {},
                    },
                },
            }
        }

        with patch.object(
            scheduler,
            "overrides_network_adjustments_materialized",
            return_value=[
                {
                    "type": "adjust_site_speed",
                    "site_name": "SiteA",
                    "download_bandwidth_mbps": 80,
                    "upload_bandwidth_mbps": 40,
                },
            ],
        ):
            changed = scheduler.apply_network_adjustments(network)

        self.assertTrue(changed)
        site = network["Root"]["children"]["SiteA"]
        self.assertEqual(site["downloadBandwidthMbps"], 80)
        self.assertEqual(site["uploadBandwidthMbps"], 40)

    def test_apply_network_adjustments_does_not_use_effective_stormguard_speeds(self):
        network = {
            "Root": {
                "downloadBandwidthMbps": 1000,
                "uploadBandwidthMbps": 1000,
                "children": {
                    "Pine Hills": {
                        "downloadBandwidthMbps": 940,
                        "uploadBandwidthMbps": 500,
                        "children": {},
                    }
                },
            }
        }

        effective_adjustments = [
            {
                "type": "adjust_site_speed",
                "site_name": "Pine Hills",
                "download_bandwidth_mbps": 4,
                "upload_bandwidth_mbps": 4,
            }
        ]

        with patch.dict(
            scheduler.apply_network_adjustments.__globals__,
            {"overrides_network_adjustments_effective": lambda: effective_adjustments},
            clear=False,
        ):
            with patch.object(
            scheduler,
            "overrides_network_adjustments_materialized",
            return_value=[],
            ) as mock_materialized:
                changed = scheduler.apply_network_adjustments(network)

        mock_materialized.assert_called_once_with()
        self.assertFalse(changed)
        site = network["Root"]["children"]["Pine Hills"]
        self.assertEqual(site["downloadBandwidthMbps"], 940)
        self.assertEqual(site["uploadBandwidthMbps"], 500)

    def test_apply_network_adjustments_does_not_materialize_runtime_treeguard_virtual_state(self):
        network = {
            "Root": {
                "children": {
                    "REGION_01": {
                        "virtual": False,
                        "children": {},
                    }
                },
            }
        }

        effective_adjustments = [
            {
                "type": "set_node_virtual",
                "node_name": "REGION_01",
                "virtual": True,
            }
        ]

        with patch.dict(
            scheduler.apply_network_adjustments.__globals__,
            {"overrides_network_adjustments_effective": lambda: effective_adjustments},
            clear=False,
        ):
            with patch.object(
                scheduler,
                "overrides_network_adjustments_materialized",
                return_value=[],
            ) as mock_materialized:
                changed = scheduler.apply_network_adjustments(network)

        mock_materialized.assert_called_once_with()
        self.assertFalse(changed)
        self.assertFalse(network["Root"]["children"]["REGION_01"]["virtual"])


if __name__ == "__main__":
    unittest.main()
