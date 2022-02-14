import time
import schedule
from datetime import date
from LibreQoS import refreshShapers
from graph import refreshGraphs
from ispConfig import graphingEnabled

if __name__ == '__main__':
	refreshShapers()
	schedule.every().day.at("04:00").do(refreshShapers)
	while True:
		schedule.run_pending()
		if graphingEnabled:
			try:
				refreshGraphs()
			except:
				print("Failed to update graphs")
		time.sleep(15) # wait one minute
