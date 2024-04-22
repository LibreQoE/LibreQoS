import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
#from graphInfluxDB import refreshBandwidthGraphs, refreshLatencyGraphs
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
	automatic_import_powercode, automatic_import_sonar
if automatic_import_splynx():
	from integrationSplynx import importFromSplynx
if automatic_import_powercode():
	from integrationPowercode import importFromPowercode
if automatic_import_sonar():
	from integrationSonar import importFromSonar
from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})

def importFromCRM():
	if automatic_import_uisp():
		try:
			import subprocess
			subprocess.run(["bin/uisp_integration"])
			# importFromUISP()
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

#def graphHandler():
#	try:
#		refreshBandwidthGraphs()
#	except:
#		print("Failed to update bandwidth graphs")
#	try:
#		refreshLatencyGraphs()
#	except:
#		print("Failed to update latency graphs")

def importAndShapeFullReload():
	importFromCRM()
	refreshShapers()

def importAndShapePartialReload():
	importFromCRM()
	refreshShapersUpdateOnly()

if __name__ == '__main__':
	importAndShapeFullReload()

	ads.add_job(importAndShapePartialReload, 'interval', minutes=queue_refresh_interval_mins(), max_instances=1)

	#if influxDBEnabled:
	#	ads.add_job(graphHandler, 'interval', seconds=10, max_instances=1)

	ads.start()
