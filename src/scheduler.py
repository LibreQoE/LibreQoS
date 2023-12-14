import time
import datetime
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
from graphInfluxDB import refreshBandwidthGraphs, refreshLatencyGraphs
from ispConfig import influxDBEnabled, automaticImportUISP, automaticImportSplynx
try:
	from ispConfig import queueRefreshIntervalMins
except:
	queueRefreshIntervalMins = 30
if automaticImportUISP:
	from integrationUISP import importFromUISP
if automaticImportSplynx:
	from integrationSplynx import importFromSplynx
try:
	from ispConfig import automaticImportPowercode
except:
	automaticImportPowercode = False
if automaticImportPowercode:
	from integrationPowercode import importFromPowercode
from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})

def importFromCRM():
	if automaticImportUISP:
		try:
			importFromUISP()
		except:
			print("Failed to import from UISP")
	elif automaticImportSplynx:
		try:
			importFromSplynx()
		except:
			print("Failed to import from Splynx")
	elif automaticImportPowercode:
		try:
			importFromPowercode()
		except:
			print("Failed to import from Powercode")

def graphHandler():
	try:
		refreshBandwidthGraphs()
	except:
		print("Failed to update bandwidth graphs")
	try:
		refreshLatencyGraphs()
	except:
		print("Failed to update latency graphs")

def importAndShapeFullReload():
	importFromCRM()
	refreshShapers()

def importAndShapePartialReload():
	importFromCRM()
	refreshShapersUpdateOnly()

if __name__ == '__main__':
	importAndShapeFullReload()

	ads.add_job(importAndShapePartialReload, 'interval', minutes=queueRefreshIntervalMins, max_instances=1)

	if influxDBEnabled:
		ads.add_job(graphHandler, 'interval', seconds=10, max_instances=1)

	ads.start()
