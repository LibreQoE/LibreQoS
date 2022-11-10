import time
import schedule
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
from graphInfluxDB import refreshBandwidthGraphs, refreshLatencyGraphs
from ispConfig import influxDBEnabled, automaticImportUISP, automaticImportSplynx
if automaticImportUISP:
	from integrationUISP import importFromUISP
if automaticImportSplynx:
	from integrationSplynx import importFromSplynx

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

def importAndShapeFullReload():
	importFromCRM()
	refreshShapers()

def importAndShapePartialReload():
	importFromCRM()
	refreshShapersUpdateOnly()

if __name__ == '__main__':
	importAndShapeFullReload()
	schedule.every().day.at("04:00").do(importAndShapeFullReload)
	schedule.every(30).minutes.do(importAndShapePartialReload)
	if influxDBEnabled:
		schedule.every(10).seconds.do(refreshBandwidthGraphs)
		schedule.every(45).seconds.do(refreshLatencyGraphs)
	while True:
		schedule.run_pending()
		time.sleep(1)
