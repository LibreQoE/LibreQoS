import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
#from graphInfluxDB import refreshBandwidthGraphs, refreshLatencyGraphs
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins
if automatic_import_uisp():
	from integrationUISP import importFromUISP
if automatic_import_splynx():
	from integrationSplynx import importFromSplynx
try:
	from ispConfig import automaticImportPowercode
except:
	automaticImportPowercode = False
if automaticImportPowercode:
	from integrationPowercode import importFromPowercode
try:
	from ispConfig import automaticImportSonar
except:
	automaticImportSonar = False
if automaticImportSonar:
	from integrationSonar import importFromSonar
from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})

def importFromCRM():
	if automatic_import_uisp():
		try:
			importFromUISP()
		except:
			print("Failed to import from UISP")
	elif automatic_import_splynx():
		try:
			importFromSplynx()
		except:
			print("Failed to import from Splynx")
	elif automaticImportPowercode:
		try:
			importFromPowercode()
		except:
			print("Failed to import from Powercode")
	elif automaticImportSonar:
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
