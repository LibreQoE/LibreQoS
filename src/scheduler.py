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

if automatic_import_splynx():
    from integrationSplynx import importFromSplynx
if automatic_import_netzur():
    from integrationNetzur import importFromNetzur
if automatic_import_powercode():
    from integrationPowercode import importFromPowercode
if automatic_import_sonar():
    from integrationSonar import importFromSonar
if automatic_import_wispgate():
    from integrationWISPGate import importFromWISPGate
from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor
import os.path

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})
network_hash = 0


def capture_output_and_run(func):
    """Wrapper function to capture stdout/stderr from a function and handle errors."""
    try:
        # Capture stdout/stderr from Python function
        old_stdout = sys.stdout
        old_stderr = sys.stderr
        captured_output = StringIO()

        sys.stdout = captured_output
        sys.stderr = captured_output

        func()  # Execute the function

        # Restore original stdout/stderr
        sys.stdout = old_stdout
        sys.stderr = old_stderr

        output = captured_output.getvalue()
        print(output)  # Print captured output
        scheduler_error(output)  # Send to error reporting

    except Exception as e:
        error_msg = f"Failed to execute function: {str(e)}"
        print(error_msg)
        scheduler_error(error_msg)

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
            # Call bin/uisp_integration with output capture
            path = get_libreqos_directory() + "/bin/uisp_integration"
            result = subprocess.run([path], capture_output=True, text=True)
            output = result.stdout + result.stderr
            print(output)  # Maintain console output
            # Report UISP output to error channel regardless of return code,
            # as UISP may signal errors in text while returning success.
            scheduler_error(output)
            blackboard_finish()
        except Exception as e:
            error_msg = f"Failed to import from UISP: {str(e)}"
            print(error_msg)
            scheduler_error(error_msg)
    elif automatic_import_splynx():
<<<<<<< HEAD
        try:
            importFromSplynx()
        except:
            print("Failed to import from Splynx")
    elif automatic_import_netzur():
        try:
            importFromNetzur()
        except:
            print("Failed to import from Netzur")
=======
        capture_output_and_run(importFromSplynx)
>>>>>>> develop
    elif automatic_import_powercode():
        capture_output_and_run(importFromPowercode)
    elif automatic_import_sonar():
        capture_output_and_run(importFromSonar)
    elif automatic_import_wispgate():
        capture_output_and_run(importFromWISPGate)
    # Post-CRM Hooks
    path = get_libreqos_directory() + "/bin/post_integration_hook.sh"
    binPath = get_libreqos_directory() + "/bin"
    if os.path.isfile(path):
        subprocess.Popen(path, cwd=binPath)


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
