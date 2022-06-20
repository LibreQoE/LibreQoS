import time
import schedule
from LibreQoS import refreshShapers
from graphBandwidth import refreshBandwidthGraphs
from graphLatency import refreshLatencyGraphs
from ispConfig import graphingEnabled, automaticImportUISP
from integrationUISP import updateFromUISP

def importandshape():
    if automaticImportUISP:
        updateFromUISP()
    refreshShapers()

if __name__ == '__main__':
    importandshape()
    schedule.every().day.at("04:00").do(importandshape)
    while True:
        schedule.run_pending()
        if graphingEnabled:
            try:
                refreshBandwidthGraphs()
                refreshLatencyGraphs(10)
            except:
                print("Failed to update graphs")
        else:
            time.sleep(60)  # wait x seconds
