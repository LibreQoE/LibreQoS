import time
import schedule
from datetime import date
from LibreQoS import refreshShapers
from graphBandwidth import refreshBandwidthGraphs
from graphLatency import refreshLatencyGraphs
from ispConfig import graphingEnabled

if __name__ == '__main__':
	refreshShapers()
	schedule.every().day.at("04:00").do(refreshShapers)
	while True:
		schedule.run_pending()
		if graphingEnabled:
			try:
				refreshBandwidthGraphs()
				refreshLatencyGraphs(10)
			except:
				print("Failed to update graphs")
		else:
			time.sleep(60) # wait x seconds
