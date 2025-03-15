from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
import sys
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
    automatic_import_powercode, automatic_import_sonar, get_libreqos_directory, \
    blackboard_finish, automatic_import_wispgate
if automatic_import_splynx():
    from integrationSplynx import importFromSplynx
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

def importFromCRM():
    if automatic_import_uisp():
        try:
            # Call bin/uisp_integration
            path = get_libreqos_directory() + "/bin/uisp_integration"
            subprocess.run([path])
            blackboard_finish()
        except Exception as e:
            print(f"Failed to import from UISP: {e}")
            return False
    elif automatic_import_splynx():
        try:
            importFromSplynx()
        except Exception as e:
            print(f"Failed to import from Splynx: {e}")
            return False
    elif automatic_import_powercode():
        try:
            importFromPowercode()
        except Exception as e:
            print(f"Failed to import from Powercode: {e}")
            return False
    elif automatic_import_sonar():
        try:
            importFromSonar()
        except Exception as e:
            print(f"Failed to import from Sonar: {e}")
            return False
    elif automatic_import_wispgate():
        try:
            importFromWISPGate()
        except Exception as e:
            print(f"Failed to import from WISPGate: {e}")
            return False
    
    # Post-CRM Hooks
    path = get_libreqos_directory() + "/bin/post_integration_hook.sh"
    binPath = get_libreqos_directory() + "/bin"
    if os.path.isfile(path):
        subprocess.Popen(path, cwd=binPath)
    
    return True

def importAndShapeFullReload():
    if importFromCRM():
        refreshShapers()
        return True
    return False

def importAndShapePartialReload():
    if importFromCRM():
        refreshShapersUpdateOnly()
        return True
    return False


    # This function is meant to be called directly from the Rust webhook
    # handler. It performs a partial reload of the configuration with CRM imports.
def handle_webhook():
    print("Starting UISP webhook handler")
    success = importAndShapePartialReload()
    print(f"UISP webhook handler completed {'successfully' if success else 'with errors'}")
    return 0 if success else 1

if __name__ == '__main__':
    if '--webhook' in sys.argv:
        sys.exit(handle_webhook())
    else:
        # Normal scheduler operation
        importAndShapeFullReload()
        ads.add_job(importAndShapePartialReload, 'interval', minutes=queue_refresh_interval_mins(), max_instances=1)
        ads.start()