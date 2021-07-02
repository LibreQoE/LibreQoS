import time
import schedule
from datetime import date
from LibreQoS import refreshShapers

if __name__ == '__main__':
	refreshShapers()
	schedule.every().day.at("04:00").do(refreshShapers)
	while True:
		schedule.run_pending()
		time.sleep(60) # wait one minute
