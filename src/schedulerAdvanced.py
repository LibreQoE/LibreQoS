import time

from LibreQoS import refreshShapers, refreshShapersUpdateOnly
from graphInfluxDB import refreshBandwidthGraphs, refreshLatencyGraphs
from ispConfig import influxDBEnabled, automaticImportUISP, automaticImportSplynx, httpRestIntegrationConfig

if automaticImportUISP:
    from integrationUISP import importFromUISP
if automaticImportSplynx:
    from integrationSplynx import importFromSplynx
if httpRestIntegrationConfig['enabled']:
    from integrationRestHttp import importFromRestHttp

from apscheduler.schedulers.background import BlockingScheduler

ads = BlockingScheduler()


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
    elif httpRestIntegrationConfig['enabled']:
        try:
            importFromRestHttp()
        except:
            print("Failed to import from RestHttp")


def importAndShapeFullReload():
    importFromCRM()
    refreshShapers()


def importAndShapePartialReload():
    importFromCRM()
    refreshShapersUpdateOnly()


if __name__ == '__main__':
    importAndShapeFullReload()
    # schedule.every().day.at("04:00").do(importAndShapeFullReload)
    ads.add_job(importAndShapeFullReload, 'cron', hour=4)

    # schedule.every(30).minutes.do(importAndShapePartialReload)
    ads.add_job(importAndShapePartialReload, 'interval', minutes=30)

    if influxDBEnabled:
        # schedule.every(10).seconds.do(refreshBandwidthGraphs)
        ads.add_job(refreshBandwidthGraphs, 'interval', seconds=10)

        # schedule.every(30).seconds.do(refreshLatencyGraphs)
        # Commented out until refreshLatencyGraphs works in v.14
        # ads.add_job(refreshLatencyGraphs, 'interval', seconds=30)

    # while True:
    # schedule.run_pending()
    # time.sleep(1)

    ads.start()
