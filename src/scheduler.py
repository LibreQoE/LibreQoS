import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
	automatic_import_powercode, automatic_import_sonar, influx_db_enabled, get_libreqos_directory, \
	blackboard_finish, blackboard_submit
if automatic_import_splynx():
	from integrationSplynx import importFromSplynx
if automatic_import_powercode():
	from integrationPowercode import importFromPowercode
if automatic_import_sonar():
	from integrationSonar import importFromSonar
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
		except:
			print("Failed to import from UISP")
	elif automatic_import_splynx():
		try:
			importFromSplynx()
		except:
			print("Failed to import from Splynx")
	elif automatic_import_powercode():
		try:
			importFromPowercode()
		except:
			print("Failed to import from Powercode")
	elif automatic_import_sonar():
		try:
			importFromSonar()
		except:
			print("Failed to import from Sonar")

	# Post-CRM Hooks
	path = get_libreqos_directory() + "/bin/post_integration_hook.sh"
	binPath = get_libreqos_directory() + "/bin"
	if os.path.isfile(path):
        	subprocess.Popen(path, cwd=binPath)

def importAndShapeFullReload():
	importFromCRM()
	refreshShapers()

def importAndShapePartialReload():
	importFromCRM()
	refreshShapersUpdateOnly()

if __name__ == '__main__':
	importAndShapeFullReload()

	ads.add_job(importAndShapePartialReload, 'interval', minutes=queue_refresh_interval_mins(), max_instances=1)

	ads.start()
