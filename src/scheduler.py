import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
import sys
from io import StringIO
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
    automatic_import_powercode, automatic_import_sonar, influx_db_enabled, get_libreqos_directory, \
    blackboard_finish, blackboard_submit, automatic_import_wispgate, enable_insight_topology, insight_topology_role, \
    automatic_import_netzur, calculate_hash, scheduler_alive, scheduler_error

from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor
import os.path

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})
network_hash = 0


def capture_output_and_run(func):
    """Capture stdout/stderr from a callable and ensure failures are non-fatal."""
    old_stdout = sys.stdout
    old_stderr = sys.stderr
    captured_output = StringIO()
    try:
        sys.stdout = captured_output
        sys.stderr = captured_output
        func()
    except BaseException as e:
        # Catch BaseException to also handle SystemExit/KeyboardInterrupt from integrations
        error_msg = f"Failed to execute function: {str(e)}"
        try:
            print(error_msg)
        finally:
            # Ensure scheduler gets error details even if printing fails
            scheduler_error(error_msg)
    finally:
        # Always restore stdio and flush captured output
        sys.stdout = old_stdout
        sys.stderr = old_stderr
        output = captured_output.getvalue()
        if output:
            print(output)
            scheduler_error(output)


def run_python_integration(module_name: str, func_name: str, label: str = ""):
    """
    Run a Python integration in a subprocess so failures cannot terminate the scheduler.
    Captures stdout/stderr, logs them, and continues regardless of exit code.
    """
    try:
        code = f"from {module_name} import {func_name} as f; f()"
        cmd = [sys.executable, "-c", code]
        result = subprocess.run(cmd, capture_output=True, text=True)
        output = (result.stdout or "") + (result.stderr or "")
        if output:
            print(output)
            scheduler_error(output)
        if result.returncode != 0:
            # Non-zero exit shouldn't stop scheduling; log and continue
            friendly = label or f"{module_name}.{func_name}"
            msg = f"Integration {friendly} exited with code {result.returncode}. Continuing."
            print(msg)
            scheduler_error(msg)
    except Exception as e:
        err = f"Failed to invoke integration {label or (module_name + '.' + func_name)}: {e}"
        print(err)
        scheduler_error(err)

def importFromCRM():
    # Check Insight Topology Status
    run_crm = True
    if enable_insight_topology() and not insight_topology_role == "Primary":
        # This node is not the primary Insight Topology node, skip CRM import
        print("Skipping CRM import as this node is not the primary Insight Topology node.")
        run_crm = False
        return
    if not run_crm:
        return

    # CRM Hooks
    if automatic_import_uisp():
        try:
            # Execute UISP integration in a subprocess and keep going on failure
            path = get_libreqos_directory() + "/bin/uisp_integration"
            result = subprocess.run([path], capture_output=True, text=True)
            output = (result.stdout or "") + (result.stderr or "")
            if output:
                print(output)
                # Report UISP output to error channel regardless of return code.
                scheduler_error(output)
            if result.returncode != 0:
                msg = f"UISP integration exited with code {result.returncode}. Continuing."
                print(msg)
                scheduler_error(msg)
            blackboard_finish()
        except Exception as e:
            error_msg = f"Failed to run UISP integration: {str(e)}"
            print(error_msg)
            scheduler_error(error_msg)
    elif automatic_import_splynx():
        run_python_integration("integrationSplynx", "importFromSplynx", label="Splynx")
    elif automatic_import_netzur():
        run_python_integration("integrationNetzur", "importFromNetzur", label="Netzur")
    elif automatic_import_powercode():
        run_python_integration("integrationPowercode", "importFromPowercode", label="Powercode")
    elif automatic_import_sonar():
        run_python_integration("integrationSonar", "importFromSonar", label="Sonar")
    elif automatic_import_wispgate():
        run_python_integration("integrationWISPGate", "importFromWISPGate", label="WISPGate")
    # Post-CRM Hooks
    path = get_libreqos_directory() + "/bin/post_integration_hook.sh"
    binPath = get_libreqos_directory() + "/bin"
    if os.path.isfile(path):
        try:
            subprocess.Popen(path, cwd=binPath)
        except Exception as e:
            msg = f"post_integration_hook.sh failed to launch: {e}"
            print(msg)
            scheduler_error(msg)


def importAndShapeFullReload():
    importFromCRM()
    if not enable_insight_topology():
        refreshShapers()


def importAndShapePartialReload():
    global network_hash

    importFromCRM()
    # Calculate if the network.json or ShapedDevices.csv has changed and reload only if it has.
    new_hash = calculate_hash()
    if new_hash != network_hash:
        refreshShapersUpdateOnly()
        network_hash = new_hash
    else:
        print("No changes detected in network.json or ShapedDevices.csv, skipping shaper refresh.")


def not_dead_yet():
    #print(f"Scheduler alive at {datetime.datetime.now()}")
    scheduler_alive()

if __name__ == '__main__':
    try:
        importAndShapeFullReload()
        network_hash = calculate_hash()

        print("Starting scheduler with jobs:")
        print(f"- not_dead_yet every 1 minute")
        refresh_interval = queue_refresh_interval_mins()
        print(f"- importAndShapePartialReload every {refresh_interval} minutes")
        
        not_dead_yet()
        ads.add_job(not_dead_yet, 'interval', minutes=1, max_instances=1)
        ads.add_job(importAndShapePartialReload, 'interval', minutes=refresh_interval, max_instances=1)

        print("Scheduler starting...")
        ads.start()
    except Exception as e:
        print(f"Error starting scheduler: {e}")
        import traceback
        traceback.print_exc()
