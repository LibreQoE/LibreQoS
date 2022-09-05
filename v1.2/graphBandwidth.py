import os
import subprocess
from subprocess import PIPE
import io
import decimal
import json
from ispConfig import fqOrCAKE, interfaceA, interfaceB, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl
from datetime import date, datetime, timedelta
import decimal
from influxdb_client import InfluxDBClient, Point, Dialect
from influxdb_client.client.write_api import SYNCHRONOUS
import dateutil.parser

def getCircuitStats(subscriberCircuits):
	interfaces = [interfaceA, interfaceB]
	for interface in interfaces:
		command = 'tc -j -s qdisc show dev ' + interface
		commands = command.split(' ')
		tcShowResults = subprocess.run(commands, stdout=subprocess.PIPE).stdout.decode('utf-8')
		if interface == interfaceA:
			interfaceAjson = json.loads(tcShowResults)
		else:
			interfaceBjson = json.loads(tcShowResults)
	for circuit in subscriberCircuits:
		if 'timeQueried' in circuit:
			circuit['priorQueryTime'] = circuit['timeQueried']
		for interface in interfaces:
			if interface == interfaceA:
				jsonVersion = interfaceAjson
			else:
				jsonVersion = interfaceBjson
			for element in jsonVersion:
				if "parent" in element:
					parentFixed = '0x' + element['parent'].split(':')[0] + ':' + '0x' + element['parent'].split(':')[1]
					if parentFixed == circuit['qdisc']:
						drops = int(element['drops'])
						packets = int(element['packets'])
						bytesSent = int(element['bytes'])
						if interface == interfaceA:
							if 'bytesSentDownload' in circuit:
								circuit['priorQueryBytesDownload'] = circuit['bytesSentDownload']
							circuit['bytesSentDownload'] = bytesSent
						else:
							if 'bytesSentUpload' in circuit:
								circuit['priorQueryBytesUpload'] = circuit['bytesSentUpload']
							circuit['bytesSentUpload'] = bytesSent
		circuit['timeQueried'] = datetime.now().isoformat()
	for circuit in subscriberCircuits:
		if 'priorQueryTime' in circuit:
			try:
				bytesDLSinceLastQuery = circuit['bytesSentDownload'] - circuit['priorQueryBytesDownload']
				bytesULSinceLastQuery = circuit['bytesSentUpload'] - circuit['priorQueryBytesUpload']
			except:
				bytesDLSinceLastQuery = 0
				bytesULSinceLastQuery = 0
			currentQueryTime = datetime.fromisoformat(circuit['timeQueried'])
			priorQueryTime = datetime.fromisoformat(circuit['priorQueryTime'])
			delta = currentQueryTime - priorQueryTime
			deltaSeconds = delta.total_seconds()
			if deltaSeconds > 0:
				bitsDownload = round((((bytesDLSinceLastQuery*8))/deltaSeconds))
				bitsUpload = round((((bytesULSinceLastQuery*8))/deltaSeconds))
			else:
				bitsDownload = 0
				bitsUpload = 0
			circuit['bitsDownloadSinceLastQuery'] = bitsDownload
			circuit['bitsUploadSinceLastQuery'] = bitsUpload
		else:
			circuit['bitsDownloadSinceLastQuery'] = 0
			circuit['bitsUploadSinceLastQuery'] = 0
	return (subscriberCircuits)

def getParentNodeStats(parentNodes, subscriberCircuits):
	for parentNode in parentNodes:
		thisNodeBitsDownload = 0
		thisNodeBitsUpload = 0
		for circuit in subscriberCircuits:
			if circuit['ParentNode'] == parentNode['parentNodeName']:
				thisNodeBitsDownload += circuit['bitsDownloadSinceLastQuery']
				thisNodeBitsUpload += circuit['bitsUploadSinceLastQuery']
		
		parentNode['bitsDownloadSinceLastQuery'] = thisNodeBitsDownload
		parentNode['bitsUploadSinceLastQuery'] = thisNodeBitsUpload
	return parentNodes

def getParentNodeDict(data, depth, parentNodeNameDict):
	if parentNodeNameDict == None:
		parentNodeNameDict = {}
	
	for elem in data:
		if 'children' in data[elem]:
			for child in data[elem]['children']:
				parentNodeNameDict[child] = elem
			tempDict = getParentNodeDict(data[elem]['children'], depth+1, parentNodeNameDict)
			parentNodeNameDict = dict(parentNodeNameDict, **tempDict)
	return parentNodeNameDict

def parentNodeNameDictPull():
	#Load network heirarchy
	with open('network.json', 'r') as j:
		network = json.loads(j.read())
	parentNodeNameDict = getParentNodeDict(network, 0, None)
	return parentNodeNameDict

def refreshBandwidthGraphs():
	startTime = datetime.now()
	with open('statsByParentNode.json', 'r') as j:
		parentNodes = json.loads(j.read())
	
	with open('statsByCircuit.json', 'r') as j:
		subscriberCircuits = json.loads(j.read())
	
	parentNodeNameDict = parentNodeNameDictPull()
	
	print("Retrieving circuit statistics")
	subscriberCircuits = getCircuitStats(subscriberCircuits)
	print("Computing parent node statistics")
	parentNodes = getParentNodeStats(parentNodes, subscriberCircuits)
	print("Writing data to InfluxDB")
	bucket = influxDBBucket
	org = influxDBOrg
	token = influxDBtoken
	url=influxDBurl
	client = InfluxDBClient(
		url=url,
		token=token,
		org=org
	)
	write_api = client.write_api(write_options=SYNCHRONOUS)
	
	queriesToSend = []
	for circuit in subscriberCircuits:
		bitsDownload = int(circuit['bitsDownloadSinceLastQuery'])
		bitsUpload = int(circuit['bitsUploadSinceLastQuery'])
		if (bitsDownload > 0) and (bitsUpload > 0):
			percentUtilizationDownload =  round((bitsDownload / round(circuit['downloadMax']*1000000)),4)
			percentUtilizationUpload =  round((bitsUpload / round(circuit['uploadMax']*1000000)),4)

			p = Point('Bandwidth').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", bitsDownload)
			queriesToSend.append(p)
			p = Point('Bandwidth').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Upload", bitsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", percentUtilizationDownload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)

	for parentNode in parentNodes:
		bitsDownload = int(parentNode['bitsDownloadSinceLastQuery'])
		bitsUpload = int(parentNode['bitsUploadSinceLastQuery'])
		if (bitsDownload > 0) and (bitsUpload > 0):
			percentUtilizationDownload =  round((bitsDownload / round(parentNode['downloadMax']*1000000)),4)
			percentUtilizationUpload =  round((bitsUpload / round(parentNode['uploadMax']*1000000)),4)
			
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload)
			queriesToSend.append(p)
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Upload", bitsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)

	write_api.write(bucket=bucket, record=queriesToSend)
	print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	client.close()
	
	with open('statsByParentNode.json', 'w') as infile:
		json.dump(parentNodes, infile)
	
	with open('statsByCircuit.json', 'w') as infile:
		json.dump(subscriberCircuits, infile)
	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(),2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")
	
if __name__ == '__main__':
	refreshBandwidthGraphs()
