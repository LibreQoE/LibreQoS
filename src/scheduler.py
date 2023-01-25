import time
import datetime
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

def graph():
	time.sleep(10)
	try:
		refreshBandwidthGraphs()
	except:
		print("Failed to run refreshBandwidthGraphs()")
	time.sleep(10)
	try:
		refreshBandwidthGraphs()
	except:
		print("Failed to run refreshBandwidthGraphs()")
	time.sleep(10)
	try:
		refreshBandwidthGraphs()
	except:
		print("Failed to run refreshBandwidthGraphs()")
	#time.sleep(1)
	#try:
	#	refreshLatencyGraphs()
	#except:
	#	print("Failed to run refreshLatencyGraphs()")

if __name__ == '__main__':
	importAndShapeFullReload()
	while True:
		finish_time = datetime.datetime.now() + datetime.timedelta(minutes=30)
		while datetime.datetime.now() < finish_time:
			if influxDBEnabled:
				graph()
			else:
				time.sleep(1)
		importAndShapePartialReload()
