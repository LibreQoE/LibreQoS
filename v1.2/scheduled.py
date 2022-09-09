import time
import schedule
from LibreQoS import refreshShapers
from graphBandwidth import refreshBandwidthGraphs
from graphLatency import refreshLatencyGraphs
from ispConfig import bandwidthGraphingEnabled, latencyGraphingEnabled, automaticImportUISP
from integrationUISP import importFromUISP

def importandshape():
	if automaticImportUISP:
		importFromUISP()
	refreshShapers()

if __name__ == '__main__':
	importandshape()
	schedule.every().day.at("04:00").do(importandshape)
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
