import importlib
import json
import os
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

_STUBBED_MODULES = (
    "LibreQoS",
    "liblqos_python",
    "apscheduler",
    "apscheduler.schedulers",
    "apscheduler.schedulers.background",
    "apscheduler.executors",
    "apscheduler.executors.pool",
)
_ORIGINAL_MODULES = {name: sys.modules.get(name) for name in _STUBBED_MODULES}
scheduler = None


def install_scheduler_stubs():
    libre = types.ModuleType("LibreQoS")
    class RefreshFailure(Exception):
        pass
    class ValidationFailure(Exception):
        pass
    libre.RefreshFailure = RefreshFailure
    libre.ValidationFailure = ValidationFailure
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
    lqlib.calculate_shaping_inputs_generation = lambda _path: "shape-1"
    lqlib.calculate_effective_network_generation = lambda _path: "effective-1"
    lqlib.topology_import_ingress_enabled = lambda: False
    lqlib.efficiency_core_ids = lambda: []
    lqlib.scheduler_alive = Mock()
    lqlib.scheduler_error = Mock()
    lqlib.scheduler_output = Mock()
    lqlib.scheduler_progress = Mock()
    lqlib.wait_for_bus_ready = Mock(return_value=True)
    lqlib.overrides_persistent_devices_effective = lambda: []
    lqlib.overrides_circuit_adjustments_effective = lambda: []
    lqlib.overrides_network_adjustments_effective = lambda: []
    lqlib.overrides_network_adjustments_materialized = lambda: []
    lqlib.overrides_materialized = lambda: {
        "persistent_devices": [],
        "circuit_adjustments": [],
        "network_adjustments": [],
    }
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

def setUpModule():
    global scheduler
    for name in ("scheduler", * _STUBBED_MODULES):
        sys.modules.pop(name, None)
    install_scheduler_stubs()
    scheduler = importlib.import_module("scheduler")


def tearDownModule():
    sys.modules.pop("scheduler", None)
    for name, module in _ORIGINAL_MODULES.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


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
        scheduler._reset_startup_topology_runtime_wait()
        scheduler._reset_partial_topology_runtime_wait()
        scheduler.clear_integration_failure()
        scheduler.shaping_runtime_hash = 0

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

    def test_ready_progress_keeps_latest_integration_failure_visible(self):
        scheduler.remember_integration_failure("Sonar exited with code 1. Continuing.")

        with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                scheduler.publish_ready_progress(
                    False,
                    "ready",
                    "Scheduler ready",
                    scheduler.SCHEDULER_REFRESH_STEP_COUNT,
                    scheduler.SCHEDULER_REFRESH_STEP_COUNT,
                    percent=100,
                )

        mock_progress.assert_called_once()
        mock_scheduler_error.assert_called_once()
        self.assertIn(
            "Scheduler ready using last-known-good topology; latest integration import failed.",
            mock_scheduler_error.call_args.args[0],
        )
        self.assertIn(
            "Sonar exited with code 1. Continuing.",
            mock_scheduler_error.call_args.args[0],
        )


class TestSchedulerLogging(unittest.TestCase):
    def setUp(self):
        scheduler.set_scheduler_status_bus_enabled(True)
        scheduler._reset_startup_topology_runtime_wait()
        scheduler._reset_partial_topology_runtime_wait()
        scheduler.clear_integration_failure()
        scheduler.shaping_runtime_hash = 0

    def test_configure_scheduler_stdio_enables_line_buffering_when_supported(self):
        class FakeStream:
            def __init__(self):
                self.calls = []

            def reconfigure(self, **kwargs):
                self.calls.append(kwargs)

        fake_stdout = FakeStream()
        fake_stderr = FakeStream()

        with patch.object(scheduler.sys, "stdout", fake_stdout):
            with patch.object(scheduler.sys, "stderr", fake_stderr):
                scheduler.configure_scheduler_stdio()

        expected = [{"line_buffering": True, "write_through": True}]
        self.assertEqual(fake_stdout.calls, expected)
        self.assertEqual(fake_stderr.calls, expected)

    def test_configure_scheduler_stdio_ignores_streams_without_reconfigure(self):
        with patch.object(scheduler.sys, "stdout", object()):
            with patch.object(scheduler.sys, "stderr", object()):
                scheduler.configure_scheduler_stdio()

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

    def test_topology_runtime_refresh_tick_waits_for_stale_shaping_inputs(self):
        stale_exc = scheduler.RefreshFailure(
            "Missing or stale shaping_inputs.json. Run topology runtime before shaping."
        )

        with patch.object(scheduler, "ensure_topology_runtime_process"):
            with patch.object(
                scheduler,
                "topology_runtime_readiness_detail",
                return_value=(True, "", "generation-1"),
            ):
                with patch.object(
                    scheduler,
                    "current_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=5):
                        with patch.object(
                            scheduler,
                            "refreshShapers",
                            side_effect=stale_exc,
                        ):
                            with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                                with patch.object(
                                    scheduler,
                                    "report_scheduler_runtime_failure",
                                ) as mock_report:
                                    with patch.object(
                                        scheduler,
                                        "publish_scheduler_progress",
                                    ) as mock_progress:
                                        with patch("builtins.print"):
                                            scheduler.shaping_runtime_hash = 1
                                            scheduler.topology_runtime_refresh_tick()

        mock_clear_error.assert_called_once()
        mock_report.assert_not_called()
        self.assertTrue(scheduler.partial_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    True,
                    "waiting_for_topology_runtime",
                    "Waiting for topology runtime",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_topology_runtime_refresh_tick_reports_validation_failure(self):
        validation_exc = scheduler.ValidationFailure("Validation failed. Will now exit.")

        with patch.object(scheduler, "ensure_topology_runtime_process"):
            with patch.object(
                scheduler,
                "topology_runtime_readiness_detail",
                return_value=(True, "", "generation-1"),
            ):
                with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=5):
                    with patch.object(
                        scheduler,
                        "refreshShapers",
                        side_effect=validation_exc,
                    ):
                        with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                            with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                                    with patch("builtins.print"):
                                        scheduler.shaping_runtime_hash = 1
                                        scheduler.topology_runtime_refresh_tick()

        mock_scheduler_error.assert_called_once_with(
            "Topology runtime refresh blocked by validation: Validation failed. Will now exit."
        )
        mock_scheduler_output.assert_called_once_with(
            "Topology runtime refresh blocked by validation: Validation failed. Will now exit."
        )
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "validation_failed",
                    "Scheduler validation failed",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_topology_runtime_refresh_tick_clears_error_after_success(self):
        with patch.object(scheduler, "ensure_topology_runtime_process"):
            with patch.object(
                scheduler,
                "topology_runtime_readiness_detail",
                return_value=(True, "", "generation-1"),
            ):
                with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=5):
                    with patch.object(scheduler, "refreshShapers") as mock_refresh:
                        with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                            with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                                scheduler.shaping_runtime_hash = 1
                                scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_called_once()
        mock_clear_error.assert_called_once()
        self.assertEqual(scheduler.shaping_runtime_hash, 5)
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "ready",
                    "Scheduler ready",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_topology_runtime_refresh_tick_records_hash_that_triggered_refresh(self):
        with patch.object(scheduler, "ensure_topology_runtime_process"):
            with patch.object(
                scheduler,
                "topology_runtime_readiness_detail",
                return_value=(True, "", "generation-1"),
            ):
                with patch.object(scheduler, "calculate_shaping_runtime_hash") as mock_hash:
                    mock_hash.side_effect = [5]
                    with patch.object(scheduler, "refreshShapers") as mock_refresh:
                        with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                            with patch.object(scheduler, "clear_scheduler_error"):
                                scheduler.shaping_runtime_hash = 1
                                scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_called_once()
        mock_scheduler_error.assert_not_called()
        self.assertEqual(scheduler.shaping_runtime_hash, 5)
        self.assertEqual(mock_hash.call_count, 1)

    def test_topology_runtime_refresh_tick_skips_until_initial_shaping_succeeds(self):
        with patch.object(scheduler, "ensure_topology_runtime_process") as mock_ensure:
            with patch.object(scheduler, "calculate_shaping_runtime_hash") as mock_hash:
                scheduler.shaping_runtime_hash = 0
                scheduler.topology_runtime_refresh_tick()

        mock_ensure.assert_not_called()
        mock_hash.assert_not_called()

    def test_topology_runtime_refresh_tick_waits_for_startup_runtime_outputs(self):
        scheduler.startup_topology_runtime_pending = True
        scheduler.startup_topology_runtime_generation = "generation-1"
        scheduler.startup_topology_runtime_started_monotonic = scheduler.time.monotonic()

        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(
                False,
                "Topology runtime is still building outputs for the current source generation.",
                "generation-1",
            ),
        ):
            with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                with patch.object(scheduler, "refreshShapers") as mock_refresh:
                    with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                        with patch("builtins.print"):
                            scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_not_called()
        mock_scheduler_error.assert_not_called()
        self.assertTrue(scheduler.startup_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    True,
                    "waiting_for_topology_runtime",
                    "Waiting for topology runtime",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_transient_topology_runtime_deferred_wait_does_not_raise_scheduler_error(self):
        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(
                False,
                "Topology runtime is still building outputs for the current source generation.",
                "generation-1",
            ),
        ):
            with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                    with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                        with patch.object(
                            scheduler,
                            "publish_scheduler_progress",
                        ) as mock_progress:
                            with patch("builtins.print"):
                                scheduler.report_topology_runtime_not_ready(
                                    "Scheduled shaping refresh deferred",
                                    phase_label="Scheduler waiting for topology runtime",
                                    step_index=3,
                                    step_count=scheduler.SCHEDULER_REFRESH_STEP_COUNT,
                                )

        mock_clear_error.assert_called_once()
        mock_scheduler_error.assert_not_called()
        mock_scheduler_output.assert_called_once()
        mock_progress.assert_called_once_with(
            False,
            "waiting_for_topology_runtime",
            "Scheduler waiting for topology runtime",
            3,
            scheduler.SCHEDULER_REFRESH_STEP_COUNT,
        )

    def test_failed_topology_runtime_deferred_wait_reports_scheduler_error(self):
        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(
                False,
                "Topology runtime failed for the current source generation: publish failed",
                "generation-1",
            ),
        ):
            with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                    with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                        with patch.object(
                            scheduler,
                            "publish_scheduler_progress",
                        ) as mock_progress:
                            with patch("builtins.print"):
                                scheduler.report_topology_runtime_not_ready(
                                    "Scheduled shaping refresh deferred",
                                    phase_label="Scheduler waiting for topology runtime",
                                    step_index=3,
                                    step_count=scheduler.SCHEDULER_REFRESH_STEP_COUNT,
                                )

        mock_clear_error.assert_not_called()
        mock_scheduler_error.assert_called_once_with(
            "Scheduled shaping refresh deferred: Topology runtime failed for the current source generation: publish failed Generation generation-1."
        )
        mock_scheduler_output.assert_called_once_with(
            "Scheduled shaping refresh deferred: Topology runtime failed for the current source generation: publish failed Generation generation-1."
        )
        mock_progress.assert_called_once_with(
            False,
            "degraded",
            "Scheduler waiting for topology runtime",
            scheduler.SCHEDULER_REFRESH_STEP_COUNT,
            scheduler.SCHEDULER_REFRESH_STEP_COUNT,
            percent=100,
        )

    def test_topology_runtime_refresh_tick_waits_for_partial_runtime_outputs(self):
        scheduler.partial_topology_runtime_pending = True
        scheduler.partial_topology_runtime_generation = "generation-1"
        scheduler.partial_topology_runtime_started_monotonic = scheduler.time.monotonic()
        scheduler.shaping_runtime_hash = 4

        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(
                False,
                "Topology runtime is still building outputs for the current source generation.",
                "generation-1",
            ),
        ):
            with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                with patch.object(scheduler, "refreshShapers") as mock_refresh:
                    with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                        with patch("builtins.print"):
                            scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_not_called()
        mock_scheduler_error.assert_not_called()
        self.assertTrue(scheduler.partial_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    True,
                    "waiting_for_topology_runtime",
                    "Waiting for topology runtime",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_topology_runtime_refresh_tick_completes_startup_when_runtime_ready(self):
        scheduler.startup_topology_runtime_pending = True
        scheduler.startup_topology_runtime_generation = "generation-1"
        scheduler.startup_topology_runtime_started_monotonic = scheduler.time.monotonic()

        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(True, "", "generation-1"),
        ):
            with patch.object(scheduler, "refreshShapers") as mock_refresh:
                with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=9) as mock_hash:
                    with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                        with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                            with patch("builtins.print"):
                                scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_called_once()
        mock_clear_error.assert_called_once()
        mock_hash.assert_called_once()
        self.assertEqual(scheduler.shaping_runtime_hash, 9)
        self.assertFalse(scheduler.startup_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "ready",
                    "Scheduler ready",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_topology_runtime_refresh_tick_completes_startup_without_refresh_for_insight_topology(self):
        scheduler.startup_topology_runtime_pending = True
        scheduler.startup_topology_runtime_generation = "generation-1"
        scheduler.startup_topology_runtime_started_monotonic = scheduler.time.monotonic()

        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(True, "", "generation-1"),
        ):
            with patch.object(scheduler, "enable_insight_topology", return_value=True):
                with patch.object(scheduler, "refreshShapers") as mock_refresh:
                    with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=9):
                        with patch.object(scheduler, "publish_scheduler_progress"):
                            with patch.object(scheduler, "clear_scheduler_error"):
                                with patch("builtins.print"):
                                    scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_not_called()
        self.assertEqual(scheduler.shaping_runtime_hash, 9)
        self.assertFalse(scheduler.startup_topology_runtime_pending)

    def test_topology_runtime_refresh_tick_completes_partial_wait_when_runtime_ready(self):
        scheduler.partial_topology_runtime_pending = True
        scheduler.partial_topology_runtime_generation = "generation-1"
        scheduler.partial_topology_runtime_started_monotonic = scheduler.time.monotonic()
        scheduler.shaping_runtime_hash = 1

        with patch.object(
            scheduler,
            "topology_runtime_readiness_detail",
            return_value=(True, "", "generation-1"),
        ):
            with patch.object(scheduler, "refreshShapers") as mock_refresh:
                with patch.object(
                    scheduler,
                    "calculate_shaping_runtime_hash",
                    return_value=9,
                ):
                    with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                        with patch.object(scheduler, "clear_scheduler_error") as mock_clear_error:
                            with patch("builtins.print"):
                                scheduler.topology_runtime_refresh_tick()

        mock_refresh.assert_called_once()
        mock_clear_error.assert_called_once()
        self.assertEqual(scheduler.shaping_runtime_hash, 9)
        self.assertFalse(scheduler.partial_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "ready",
                    "Scheduler ready",
                )
                for call in mock_progress.call_args_list
            )
        )

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

    def test_import_and_shape_full_reload_returns_false_when_topology_runtime_not_ready(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(
                    scheduler,
                    "topology_runtime_readiness_detail",
                    return_value=(
                        False,
                        "Topology runtime is still building outputs for the current source generation.",
                        "generation-1",
                    ),
                ):
                    with patch.object(scheduler, "publish_scheduler_progress"):
                        self.assertFalse(scheduler.importAndShapeFullReload())
        self.assertTrue(scheduler.startup_topology_runtime_pending)

    def test_run_scheduler_main_reports_validation_failure_without_ready(self):
        fake_ads = Mock()
        validation_exc = scheduler.ValidationFailure("Validation failed. Because this is not the first run since boot (queues already set up) - will now exit.")

        with patch.object(scheduler, "ads", fake_ads):
            with patch.object(scheduler, "ensure_bus_ready"):
                with patch.object(
                    scheduler,
                    "importAndShapeFullReload",
                    side_effect=validation_exc,
                ):
                    with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                        with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                            with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                                with patch.object(scheduler, "not_dead_yet"):
                                    with patch("builtins.print"):
                                        scheduler.run_scheduler_main()

        mock_scheduler_error.assert_called_once_with(
            "Scheduler startup shaping refresh blocked by validation: Validation failed. Because this is not the first run since boot (queues already set up) - will now exit."
        )
        mock_scheduler_output.assert_called_once_with(
            "Scheduler startup shaping refresh blocked by validation: Validation failed. Because this is not the first run since boot (queues already set up) - will now exit."
        )
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "validation_failed",
                    "Scheduler validation failed",
                )
                for call in mock_progress.call_args_list
            )
        )
        self.assertFalse(
            any(
                call.args[:3] == (
                    False,
                    "ready",
                    "Scheduler ready",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_partial_reload_reports_validation_failure(self):
        validation_exc = scheduler.ValidationFailure("Validation failed. Will now exit.")

        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=True):
                with patch.object(scheduler, "calculate_shaping_runtime_hash", side_effect=[2]):
                    with patch.object(scheduler, "shaping_runtime_hash", 1):
                        with patch.object(scheduler, "refreshShapers", side_effect=validation_exc):
                            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                    with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                                        scheduler.importAndShapePartialReload()

        mock_scheduler_error.assert_called_once_with(
            "Scheduled shaping refresh blocked by validation: Validation failed. Will now exit."
        )
        mock_scheduler_output.assert_called_once_with(
            "Scheduled shaping refresh blocked by validation: Validation failed. Will now exit."
        )
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "validation_failed",
                    "Scheduler validation failed",
                )
                for call in mock_progress.call_args_list
            )
        )

    def test_partial_reload_waits_for_stale_shaping_inputs(self):
        stale_exc = scheduler.RefreshFailure(
            "Missing or stale shaping_inputs.json. Run topology runtime before shaping."
        )

        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=True):
                with patch.object(scheduler, "calculate_shaping_runtime_hash", return_value=2):
                    with patch.object(scheduler, "shaping_runtime_hash", 1):
                        with patch.object(
                            scheduler,
                            "topology_runtime_readiness_detail",
                            return_value=(True, "", "generation-1"),
                        ):
                            with patch.object(
                                scheduler,
                                "current_topology_source_generation",
                                return_value="generation-1",
                            ):
                                with patch.object(
                                    scheduler,
                                    "refreshShapers",
                                    side_effect=stale_exc,
                                ):
                                    with patch.object(
                                        scheduler,
                                        "clear_scheduler_error",
                                    ) as mock_clear_error:
                                        with patch.object(
                                            scheduler,
                                            "report_scheduler_runtime_failure",
                                        ) as mock_report:
                                            with patch.object(
                                                scheduler,
                                                "publish_scheduler_progress",
                                            ) as mock_progress:
                                                with patch("builtins.print"):
                                                    scheduler.importAndShapePartialReload()

        mock_clear_error.assert_called_once()
        mock_report.assert_not_called()
        self.assertTrue(scheduler.partial_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    True,
                    "waiting_for_topology_runtime",
                    "Waiting for topology runtime",
                )
                for call in mock_progress.call_args_list
            )
        )


class TestTopologyRuntimeReadiness(unittest.TestCase):
    def _write_ready_runtime_status(self, tempdir):
        shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
        effective_network = os.path.join(tempdir, "network.effective.json")
        topology_state = os.path.join(tempdir, "state", "topology")
        os.makedirs(topology_state, exist_ok=True)
        with open(shaping_inputs, "w", encoding="utf-8") as handle:
            handle.write("{}\n")
        with open(effective_network, "w", encoding="utf-8") as handle:
            handle.write("{}\n")
        with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
            json.dump(
                {
                    "source_generation": "generation-1",
                    "shaping_generation": "shape-1",
                    "effective_generation": "effective-1",
                    "shaping_inputs_path": shaping_inputs,
                    "effective_network_path": effective_network,
                    "ready": True,
                },
                handle,
            )
        return shaping_inputs, effective_network

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
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
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
            shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
            effective_network = os.path.join(tempdir, "network.effective.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(shaping_inputs, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(effective_network, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "shaping_generation": "shape-1",
                        "effective_generation": "effective-1",
                        "shaping_inputs_path": shaping_inputs,
                        "effective_network_path": effective_network,
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

    def test_ready_true_without_shaping_generation_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
            effective_network = os.path.join(tempdir, "network.effective.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(shaping_inputs, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(effective_network, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "effective_network_path": effective_network,
                        "shaping_inputs_path": shaping_inputs,
                        "ready": True,
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
        self.assertIn("has not published shaping inputs", detail)

    def test_ready_true_without_shaping_inputs_file_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            missing_path = os.path.join(tempdir, "shaping_inputs.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "shaping_generation": "shape-1",
                        "shaping_inputs_path": missing_path,
                        "ready": True,
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
        self.assertIn("shaping inputs are not available", detail)

    def test_ready_true_with_stale_shaping_inputs_generation_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
            effective_network = os.path.join(tempdir, "network.effective.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(shaping_inputs, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(effective_network, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "shaping_generation": "shape-1",
                        "effective_generation": "effective-1",
                        "shaping_inputs_path": shaping_inputs,
                        "effective_network_path": effective_network,
                        "ready": True,
                    },
                    handle,
                )
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_shaping_inputs_generation",
                        return_value="shape-2",
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("do not match", detail)

    def test_ready_true_with_stale_effective_generation_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
            effective_network = os.path.join(tempdir, "network.effective.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(shaping_inputs, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(effective_network, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "shaping_generation": "shape-1",
                        "effective_generation": "effective-1",
                        "shaping_inputs_path": shaping_inputs,
                        "effective_network_path": effective_network,
                        "ready": True,
                    },
                    handle,
                )
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_effective_network_generation",
                        return_value="effective-2",
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("effective network does not match", detail)

    def test_ready_true_without_effective_network_path_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            shaping_inputs = os.path.join(tempdir, "shaping_inputs.json")
            topology_state = os.path.join(tempdir, "state", "topology")
            os.makedirs(topology_state, exist_ok=True)
            with open(shaping_inputs, "w", encoding="utf-8") as handle:
                handle.write("{}\n")
            with open(os.path.join(topology_state, "topology_runtime_status.json"), "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "source_generation": "generation-1",
                        "shaping_generation": "shape-1",
                        "shaping_inputs_path": shaping_inputs,
                        "ready": True,
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
        self.assertIn("effective network path", detail)

    def test_effective_generation_exception_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            self._write_ready_runtime_status(tempdir)
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_effective_network_generation",
                        side_effect=OSError("bad effective network"),
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("effective network generation could not be verified", detail)
        self.assertIn("bad effective network", detail)

    def test_effective_generation_none_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            self._write_ready_runtime_status(tempdir)
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_effective_network_generation",
                        return_value=None,
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("effective network generation could not be verified", detail)

    def test_shaping_generation_exception_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            self._write_ready_runtime_status(tempdir)
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_shaping_inputs_generation",
                        side_effect=OSError("bad shaping inputs"),
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("shaping inputs generation could not be verified", detail)
        self.assertIn("bad shaping inputs", detail)

    def test_shaping_generation_none_is_not_ready(self):
        with tempfile.TemporaryDirectory() as tempdir:
            self._write_ready_runtime_status(tempdir)
            with patch.object(scheduler, "get_libreqos_directory", return_value=tempdir):
                with patch.object(
                    scheduler,
                    "calculate_topology_source_generation",
                    return_value="generation-1",
                ):
                    with patch.object(
                        scheduler,
                        "calculate_shaping_inputs_generation",
                        return_value=None,
                    ):
                        ready, detail, generation = scheduler.topology_runtime_readiness_detail()

        self.assertFalse(ready)
        self.assertEqual(generation, "generation-1")
        self.assertIn("shaping inputs generation could not be verified", detail)


class TestTopologyRuntimeGating(unittest.TestCase):
    def setUp(self):
        scheduler._reset_startup_topology_runtime_wait()
        scheduler._reset_partial_topology_runtime_wait()
        scheduler.clear_integration_failure()
        scheduler.shaping_runtime_hash = 0

    def test_full_reload_skips_refresh_when_topology_runtime_not_ready(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(scheduler, "report_topology_runtime_not_ready") as mock_report:
                    with patch.object(scheduler, "refreshShapers") as mock_refresh:
                        with patch.object(scheduler, "publish_scheduler_progress"):
                            scheduler.importAndShapeFullReload()

        mock_refresh.assert_not_called()
        mock_report.assert_not_called()
        self.assertTrue(scheduler.startup_topology_runtime_pending)

    def test_partial_reload_waits_when_topology_runtime_not_ready(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(
                    scheduler,
                    "topology_runtime_readiness_detail",
                    return_value=(
                        False,
                        "Topology runtime is still building outputs for the current source generation.",
                        "generation-1",
                    ),
                ):
                    with patch.object(scheduler, "report_topology_runtime_not_ready") as mock_report:
                        with patch.object(scheduler, "refreshShapers") as mock_refresh:
                            with patch.object(scheduler, "publish_scheduler_progress"):
                                scheduler.importAndShapePartialReload()

        mock_refresh.assert_not_called()
        mock_report.assert_not_called()
        self.assertTrue(scheduler.partial_topology_runtime_pending)

    def test_partial_reload_reports_runtime_failure_when_topology_runtime_failed(self):
        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=False):
                with patch.object(
                    scheduler,
                    "topology_runtime_readiness_detail",
                    return_value=(
                        False,
                        "Topology runtime failed for the current source generation: publish failed",
                        "generation-1",
                    ),
                ):
                    with patch.object(scheduler, "report_topology_runtime_not_ready") as mock_report:
                        with patch.object(scheduler, "refreshShapers") as mock_refresh:
                            with patch.object(scheduler, "publish_scheduler_progress"):
                                scheduler.importAndShapePartialReload()

        mock_refresh.assert_not_called()
        mock_report.assert_called_once()

    def test_full_reload_waits_for_stale_shaping_inputs(self):
        stale_exc = scheduler.RefreshFailure(
            "Missing or stale shaping_inputs.json. Run topology runtime before shaping."
        )

        with patch.object(scheduler, "importFromCRM"):
            with patch.object(scheduler, "ensure_topology_runtime_process", return_value=True):
                with patch.object(scheduler, "enable_insight_topology", return_value=False):
                    with patch.object(
                        scheduler,
                        "topology_runtime_readiness_detail",
                        return_value=(True, "", "generation-1"),
                    ):
                        with patch.object(
                            scheduler,
                            "current_topology_source_generation",
                            return_value="generation-1",
                        ):
                            with patch.object(
                                scheduler,
                                "refreshShapers",
                                side_effect=stale_exc,
                            ):
                                with patch.object(
                                    scheduler,
                                    "clear_scheduler_error",
                                ) as mock_clear_error:
                                    with patch.object(
                                        scheduler,
                                        "publish_scheduler_progress",
                                    ) as mock_progress:
                                        with patch("builtins.print"):
                                            ready = scheduler.importAndShapeFullReload()

        self.assertFalse(ready)
        mock_clear_error.assert_called_once()
        self.assertTrue(scheduler.startup_topology_runtime_pending)
        self.assertTrue(
            any(
                call.args[:3] == (
                    True,
                    "waiting_for_topology_runtime",
                    "Waiting for topology runtime",
                )
                for call in mock_progress.call_args_list
            )
        )


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

    def test_required_override_section_retries_transient_lock_failure(self):
        reader = Mock(side_effect=[
            RuntimeError("locked by another process"),
            RuntimeError("still locked by another process"),
            [{"type": "adjust_site_speed"}],
        ])

        with patch.object(scheduler, "OVERRIDE_SECTION_READ_ATTEMPTS", 5):
            with patch.object(scheduler, "OVERRIDE_SECTION_READ_RETRY_SECONDS", 0.01):
                with patch.object(scheduler.time, "sleep") as mock_sleep:
                    adjustments = scheduler.read_required_override_section(
                        "network adjustments",
                        reader,
                    )

        self.assertEqual(adjustments, [{"type": "adjust_site_speed"}])
        self.assertEqual(reader.call_count, 3)
        self.assertEqual(mock_sleep.call_count, 2)

    def test_required_override_section_raises_after_retry_budget(self):
        reader = Mock(side_effect=RuntimeError("locked by another process: pid 42"))

        with patch.object(scheduler, "OVERRIDE_SECTION_READ_ATTEMPTS", 2):
            with patch.object(scheduler.time, "sleep"):
                with self.assertRaises(scheduler.RequiredOverrideReadError) as raised:
                    scheduler.read_required_override_section(
                        "network adjustments",
                        reader,
                    )

        self.assertEqual(reader.call_count, 2)
        self.assertIn("failed to read network adjustments after 2 attempts", str(raised.exception))
        self.assertIn("locked by another process: pid 42", str(raised.exception))

    def test_required_override_section_does_not_retry_non_lock_failure(self):
        reader = Mock(side_effect=RuntimeError("invalid overrides json"))

        with patch.object(scheduler, "OVERRIDE_SECTION_READ_ATTEMPTS", 5):
            with patch.object(scheduler.time, "sleep") as mock_sleep:
                with self.assertRaises(RuntimeError) as raised:
                    scheduler.read_required_override_section(
                        "network adjustments",
                        reader,
                    )

        self.assertEqual(reader.call_count, 1)
        mock_sleep.assert_not_called()
        self.assertIn("invalid overrides json", str(raised.exception))

    def test_apply_lqos_overrides_aborts_when_network_adjustments_are_unavailable(self):
        with patch.object(scheduler, "topology_import_ingress_enabled", return_value=True):
            with patch.object(
                scheduler,
                "overrides_materialized",
                side_effect=RuntimeError("locked by another process: pid 42"),
            ):
                with patch.object(scheduler, "OVERRIDE_SECTION_READ_ATTEMPTS", 2):
                    with patch.object(scheduler.time, "sleep"):
                        with patch.object(scheduler, "load_topology_canonical_state") as mock_load_canonical:
                            with patch.object(scheduler, "write_topology_canonical_state") as mock_write_canonical:
                                with self.assertRaises(scheduler.RequiredOverrideReadError):
                                    scheduler.apply_lqos_overrides()

        mock_load_canonical.assert_not_called()
        mock_write_canonical.assert_not_called()

    def test_apply_lqos_overrides_reads_all_sections_before_writing_non_ingress_files(self):
        with patch.object(scheduler, "topology_import_ingress_enabled", return_value=False):
            with patch.object(
                scheduler,
                "overrides_materialized",
                side_effect=RuntimeError("locked by another process: pid 42"),
            ):
                with patch.object(scheduler, "OVERRIDE_SECTION_READ_ATTEMPTS", 2):
                    with patch.object(scheduler.time, "sleep"):
                        with patch.object(scheduler, "read_shaped_devices_csv") as mock_read_sd:
                            with patch.object(scheduler, "write_shaped_devices_csv") as mock_write_sd:
                                with patch.object(scheduler, "load_network_json") as mock_load_network:
                                    with patch.object(scheduler, "write_network_json") as mock_write_network:
                                        with self.assertRaises(scheduler.RequiredOverrideReadError):
                                            scheduler.apply_lqos_overrides()

        mock_read_sd.assert_not_called()
        mock_write_sd.assert_not_called()
        mock_load_network.assert_not_called()
        mock_write_network.assert_not_called()

    def test_apply_lqos_overrides_uses_single_materialized_snapshot(self):
        header = [
            "Circuit ID", "Circuit Name", "Device ID", "Device Name", "Parent Node", "MAC",
            "IPv4", "IPv6", "Download Min Mbps", "Upload Min Mbps", "Download Max Mbps",
            "Upload Max Mbps", "Comment",
        ]
        rows = []
        materialized = {
            "persistent_devices": [],
            "circuit_adjustments": [],
            "network_adjustments": [],
        }

        with patch.object(scheduler, "topology_import_ingress_enabled", return_value=False):
            with patch.object(scheduler, "overrides_materialized", return_value=materialized) as mock_materialized:
                with patch.object(
                    scheduler,
                    "overrides_persistent_devices_materialized",
                    side_effect=AssertionError("legacy persistent reader should not be used"),
                    create=True,
                ) as mock_persistent:
                    with patch.object(
                        scheduler,
                        "overrides_circuit_adjustments_materialized",
                        side_effect=AssertionError("legacy circuit reader should not be used"),
                        create=True,
                    ) as mock_circuit:
                        with patch.object(
                            scheduler,
                            "overrides_network_adjustments_materialized",
                            side_effect=AssertionError("legacy network reader should not be used"),
                        ) as mock_network:
                            with patch.object(scheduler, "read_shaped_devices_csv", return_value=(header, rows)):
                                with patch.object(scheduler, "load_network_json", return_value={}):
                                    with patch.object(scheduler, "load_topology_canonical_state", return_value=None):
                                        scheduler.apply_lqos_overrides()

        mock_materialized.assert_called_once_with()
        mock_persistent.assert_not_called()
        mock_circuit.assert_not_called()
        mock_network.assert_not_called()

    def test_import_from_crm_does_not_continue_after_override_failure(self):
        with patch.object(
            scheduler,
            "apply_lqos_overrides",
            side_effect=scheduler.RequiredOverrideReadError("locked by pid 42"),
        ):
            with patch.object(scheduler.os.path, "isfile", return_value=False):
                with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                    with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                        with patch.object(scheduler, "publish_scheduler_progress") as mock_progress:
                            with patch("builtins.print"):
                                with self.assertRaises(scheduler.RequiredOverrideReadError):
                                    scheduler.importFromCRM()

        self.assertIn(
            "preserving last-known-good topology",
            mock_scheduler_error.call_args.args[0],
        )
        self.assertEqual(mock_scheduler_output.call_args.args, mock_scheduler_error.call_args.args)
        self.assertTrue(
            any(
                call.args[:3] == (
                    False,
                    "degraded",
                    "Scheduler blocked by override failure",
                )
                for call in mock_progress.call_args_list
            )
        )

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
                with patch.object(
                    scheduler,
                    "overrides_materialized",
                    return_value={
                        "persistent_devices": [],
                        "circuit_adjustments": [{
                            "type": "device_adjust_sqm",
                            "device_id": "splynx_service_93",
                            "sqm_override": "fq_codel/fq_codel",
                        }],
                        "network_adjustments": [],
                    },
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
                with patch.object(
                    scheduler,
                    "overrides_materialized",
                    return_value={
                        "persistent_devices": [],
                        "circuit_adjustments": [{
                            "type": "reparent_circuit",
                            "circuit_id": "93",
                            "parent_node": "AP-Updated",
                        }],
                        "network_adjustments": [],
                    },
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
                with patch.object(
                    scheduler,
                    "overrides_materialized",
                    return_value={
                        "persistent_devices": [],
                        "circuit_adjustments": [],
                        "network_adjustments": [{
                                "type": "adjust_site_speed",
                                "node_id": "node-b",
                                "site_name": "NodeB",
                                "download_bandwidth_mbps": 80,
                                "upload_bandwidth_mbps": 40,
                            }],
                    },
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
        rate_input = canonical_state["nodes"][0]["rate_input"]
        self.assertEqual(rate_input["source"], "operator_override")
        self.assertEqual(rate_input["intrinsic_download_mbps"], 80)
        self.assertEqual(rate_input["intrinsic_upload_mbps"], 40)

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

    def test_runtime_state_path_includes_shaping_inputs(self):
        # Test-only fake install root.
        with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):  # nosec B108
            shaping_inputs_path = scheduler.get_runtime_state_path("shaping", "shaping_inputs.json")

        self.assertEqual(
            shaping_inputs_path,
            "/tmp/libreqos/state/shaping/shaping_inputs.json",
        )  # nosec B108

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
