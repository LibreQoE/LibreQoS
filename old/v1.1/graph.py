import os
import subprocess
from subprocess import PIPE
import io
import decimal
import json
from operator import itemgetter 
from prettytable import PrettyTable
from ispConfig import fqOrCAKE, interfaceA, interfaceB, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl
from datetime import date, datetime, timedelta
import decimal
from itertools import groupby
from influxdb_client import InfluxDBClient, Point, Dialect
from influxdb_client.client.write_api import SYNCHRONOUS
import dateutil.parser

def getDeviceStats(devices):
	interfaces = [interfaceA, interfaceB]
	for interface in interfaces:
		command = 'tc -j -s qdisc show dev ' + interface
		commands = command.split(' ')
		tcShowResults = subprocess.run(commands, stdout=subprocess.PIPE).stdout.decode('utf-8')
		if interface == interfaceA:
			interfaceAjson = json.loads(tcShowResults)
		else:
			interfaceBjson = json.loads(tcShowResults)
	for device in devices:
		if 'timeQueried' in device:
			device['priorQueryTime'] = device['timeQueried']
		for interface in interfaces:
			if interface == interfaceA:
				jsonVersion = interfaceAjson
			else:
				jsonVersion = interfaceBjson
			for element in jsonVersion:
				if "parent" in element:
					if element['parent'] == device['qdisc']:
						drops = int(element['drops'])
						packets = int(element['packets'])
						bytesSent = int(element['bytes'])
						if interface == interfaceA:
							if 'bytesSentDownload' in device:
								device['priorQueryBytesDownload'] = device['bytesSentDownload']
							device['bytesSentDownload'] = bytesSent
						else:
							if 'bytesSentUpload' in device:
								device['priorQueryBytesUpload'] = device['bytesSentUpload']
							device['bytesSentUpload'] = bytesSent
		device['timeQueried'] = datetime.now().isoformat()
	for device in devices:
		if 'priorQueryTime' in device:
			bytesDLSinceLastQuery = device['bytesSentDownload'] - device['priorQueryBytesDownload']
			bytesULSinceLastQuery = device['bytesSentUpload'] - device['priorQueryBytesUpload']
			currentQueryTime = datetime.fromisoformat(device['timeQueried'])
			priorQueryTime = datetime.fromisoformat(device['priorQueryTime'])
			delta = currentQueryTime - priorQueryTime
			deltaSeconds = delta.total_seconds()
			if deltaSeconds > 0:
				mbpsDownload = ((bytesDLSinceLastQuery/125000))/deltaSeconds
				mbpsUpload = ((bytesULSinceLastQuery/125000))/deltaSeconds
			else:
				mbpsDownload = 0
				mbpsUpload = 0
			device['mbpsDownloadSinceLastQuery'] = mbpsDownload
			device['mbpsUploadSinceLastQuery'] = mbpsUpload
		else:
			device['mbpsDownloadSinceLastQuery'] = 0
			device['mbpsUploadSinceLastQuery'] = 0
	return (devices)

def getParentNodeStats(parentNodes, devices):
	for parentNode in parentNodes:
		thisNodeMbpsDownload = 0
		thisNodeMbpsUpload = 0
		for device in devices:
			if device['ParentNode'] == parentNode['parentNodeName']:
				thisNodeMbpsDownload += device['mbpsDownloadSinceLastQuery']
				thisNodeMbpsUpload += device['mbpsUploadSinceLastQuery']
		parentNode['mbpsDownloadSinceLastQuery'] = thisNodeMbpsDownload
		parentNode['mbpsUploadSinceLastQuery'] = thisNodeMbpsUpload
	return parentNodes

def refreshGraphs():
	startTime = datetime.now()
	with open('statsByParentNode.json', 'r') as j:
		parentNodes = json.loads(j.read())
	
	with open('statsByDevice.json', 'r') as j:
		devices = json.loads(j.read())
	
	print("Retrieving device statistics")
	devices = getDeviceStats(devices)
	print("Computing parent node statistics")
	parentNodes = getParentNodeStats(parentNodes, devices)
	print("Writing data to InfluxDB")
	bucket = influxDBBucket
	org = influxDBOrg
	token = influxDBtoken
	url = influxDBurl
	client = InfluxDBClient(
		url=url,
		token=token,
		org=org
	)
	write_api = client.write_api(write_options=SYNCHRONOUS)
	
	queriesToSend = []
	for device in devices:
		mbpsDownload = float(device['mbpsDownloadSinceLastQuery'])
		mbpsUpload = float(device['mbpsUploadSinceLastQuery'])
		if (mbpsDownload > 0) and (mbpsUpload > 0):
			percentUtilizationDownload =  float(mbpsDownload / device['downloadMax'])
			percentUtilizationUpload =  float(mbpsUpload / device['uploadMax'])
			
			p = Point('Bandwidth').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).field("Download", mbpsDownload)
			queriesToSend.append(p)
			p = Point('Bandwidth').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).field("Upload", mbpsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).field("Download", percentUtilizationDownload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)

	for parentNode in parentNodes:
		mbpsDownload = float(parentNode['mbpsDownloadSinceLastQuery'])
		mbpsUpload = float(parentNode['mbpsUploadSinceLastQuery'])
		if (mbpsDownload > 0) and (mbpsUpload > 0):
			percentUtilizationDownload =  float(mbpsDownload / parentNode['downloadMax'])
			percentUtilizationUpload =  float(mbpsUpload / parentNode['uploadMax'])
			
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).field("Download", mbpsDownload)
			queriesToSend.append(p)
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).field("Upload", mbpsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).field("Download", percentUtilizationDownload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).field("Upload", percentUtilizationUpload)

	write_api.write(bucket=bucket, record=queriesToSend)
	print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	client.close()
	
	with open('statsByParentNode.json', 'w') as infile:
		json.dump(parentNodes, infile)
	
	with open('statsByDevice.json', 'w') as infile:
		json.dump(devices, infile)
	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds())
	print("Graphs updated within " + str(durationSeconds) + " seconds.")
	
if __name__ == '__main__':
	refreshGraphs()
