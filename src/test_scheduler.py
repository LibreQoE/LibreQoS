import importlib
import sys
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
    lqlib.get_libreqos_directory = lambda: "/tmp/libreqos"
    lqlib.blackboard_finish = Mock()
    lqlib.blackboard_submit = Mock()
    lqlib.automatic_import_wispgate = lambda: False
    lqlib.enable_insight_topology = lambda: False
    lqlib.insight_topology_role = lambda: "primary"
    lqlib.automatic_import_netzur = lambda: False
    lqlib.automatic_import_visp = lambda: False
    lqlib.calculate_hash = lambda: 0
    lqlib.efficiency_core_ids = lambda: []
    lqlib.scheduler_alive = Mock()
    lqlib.scheduler_error = Mock()
    lqlib.scheduler_output = Mock()
    lqlib.overrides_persistent_devices_effective = lambda: []
    lqlib.overrides_circuit_adjustments = lambda: []
    lqlib.overrides_network_adjustments_effective = lambda: []
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
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):
                with patch.object(scheduler, "run_integration_subprocess", return_value=result) as mock_run:
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=True):
                            with patch.object(scheduler.subprocess, "Popen") as mock_popen:
                                scheduler.importFromCRM()

        mock_run.assert_called_once()
        mock_popen.assert_called_once_with(
            "/tmp/libreqos/bin/post_integration_hook.sh",
            cwd="/tmp/libreqos/bin",
        )


class TestSchedulerErrorReporting(unittest.TestCase):
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
        mock_scheduler_output.assert_called_once_with("normal info\n")

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

        mock_scheduler_error.assert_called_once_with(
            "Integration Example exited with code 2. Continuing."
        )
        mock_scheduler_output.assert_called_once_with("normal info\n")

    def test_import_from_crm_clears_error_and_keeps_success_output_non_error(self):
        result = types.SimpleNamespace(returncode=0, stdout="uisp info\n", stderr="")

        with patch.object(scheduler, "automatic_import_uisp", return_value=True):
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):
                with patch.object(scheduler, "run_integration_subprocess", return_value=result):
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=False):
                            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                    with patch("builtins.print"):
                                        scheduler.importFromCRM()

        self.assertEqual(mock_scheduler_error.call_args_list, [(( "",),)])
        self.assertEqual(
            mock_scheduler_output.call_args_list,
            [(( "",),), (("uisp info\n",),)],
        )

    def test_import_from_crm_reports_nonzero_exit(self):
        result = types.SimpleNamespace(returncode=1, stdout="uisp info\n", stderr="")

        with patch.object(scheduler, "automatic_import_uisp", return_value=True):
            with patch.object(scheduler, "get_libreqos_directory", return_value="/tmp/libreqos"):
                with patch.object(scheduler, "run_integration_subprocess", return_value=result):
                    with patch.object(scheduler, "apply_lqos_overrides"):
                        with patch.object(scheduler.os.path, "isfile", return_value=False):
                            with patch.object(scheduler, "scheduler_error") as mock_scheduler_error:
                                with patch.object(scheduler, "scheduler_output") as mock_scheduler_output:
                                    with patch("builtins.print"):
                                        scheduler.importFromCRM()

        self.assertEqual(
            mock_scheduler_error.call_args_list,
            [(( "",),), (("UISP integration exited with code 1. Continuing.",),)],
        )
        self.assertEqual(
            mock_scheduler_output.call_args_list,
            [(( "",),), (("uisp info\n",),)],
        )


if __name__ == "__main__":
    unittest.main()
