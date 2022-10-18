import time
import schedule
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
from graphBandwidth import refreshBandwidthGraphs
from graphLatency import refreshLatencyGraphs
from ispConfig import bandwidthGraphingEnabled, latencyGraphingEnabled, automaticImportUISP
if automaticImportUISP:
	from integrationUISP import importFromUISP

def importAndShapeFullReload():
	if automaticImportUISP:
		try:
			importFromUISP()
		except:
			print("Failed to import from UISP")
	refreshShapers()

def importAndShapePartialReload():
	if automaticImportUISP:
		try:
			importFromUISP()
		except:
			print("Failed to import from UISP")
	refreshShapersUpdateOnly()

if __name__ == '__main__':
	importAndShapeFullReload()
	schedule.every().day.at("04:00").do(importAndShapeFullReload)
	schedule.every(30).minutes.do(importAndShapePartialReload)
	while True:
		schedule.run_pending()
		if bandwidthGraphingEnabled:
			try:
				refreshBandwidthGraphs()
			except:
				print("Failed to update bandwidth graphs")
		if latencyGraphingEnabled:
			try:
				refreshLatencyGraphs(10)
			except:
				print("Failed to update latency graphs")
		else:
			time.sleep(10)
