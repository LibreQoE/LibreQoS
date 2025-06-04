import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
	automatic_import_powercode, automatic_import_sonar, influx_db_enabled, get_libreqos_directory, \
	blackboard_finish, blackboard_submit, automatic_import_wispgate, enable_insight_topology, insight_topology_role, \
	calculate_hash
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
network_hash = 0

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
	elif automatic_import_wispgate():
		try:
			importFromWISPGate()
		except:
			print("Failed to import from WISPGate")
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

if __name__ == '__main__':
	importAndShapeFullReload()
	network_hash = calculate_hash()

	ads.add_job(importAndShapePartialReload, 'interval', minutes=queue_refresh_interval_mins(), max_instances=1)

	ads.start()
