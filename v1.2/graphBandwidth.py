import subprocess
import json
import subprocess
from datetime import datetime

from influxdb_client import InfluxDBClient, Point
from influxdb_client.client.write_api import SYNCHRONOUS

from ispConfig import interfaceA, interfaceB, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl


def getInterfaceStats(interface):
	command = 'tc -j -s qdisc show dev ' + interface
	jsonAr = json.loads(subprocess.run(command.split(' '), stdout=subprocess.PIPE).stdout.decode('utf-8'))
	jsonDict = {}
	for element in filter(lambda e: 'parent' in e, jsonAr):
		flowID = ':'.join(map(lambda p: f'0x{p}', element['parent'].split(':')[0:2]))
		jsonDict[flowID] = element
	del jsonAr
	return jsonDict


def chunk_list(l, n):
	for i in range(0, len(l), n):
		yield l[i:i + n]


def getsubscriberCircuitstats(subscriberCircuits):
	interfaces = [interfaceA, interfaceB]
	ifaceStats = list(map(getInterfaceStats, interfaces))

	for circuit in subscriberCircuits:
		if 'timeQueried' in circuit:
			circuit['priorQueryTime'] = circuit['timeQueried']
		for (interface, stats, dirSuffix) in zip(interfaces, ifaceStats, ['Download', 'Upload']):

			element = stats[circuit['qdisc']] if circuit['qdisc'] in stats else False

			if element:

				bytesSent = float(element['bytes'])
				drops = float(element['drops'])
				packets = float(element['packets'])

				if 'bytesSent' + dirSuffix in circuit:
					circuit['priorQueryBytes' + dirSuffix] = circuit['bytesSent' + dirSuffix]
				circuit['bytesSent' + dirSuffix] = bytesSent

				if 'dropsSent' + dirSuffix in circuit:
					circuit['priorQueryDrops' + dirSuffix] = circuit['dropsSent' + dirSuffix]
				circuit['dropsSent' + dirSuffix] = drops

				if 'packetsSent' + dirSuffix in circuit:
					circuit['priorPacketsSent' + dirSuffix] = circuit['packetsSent' + dirSuffix]
				circuit['packetsSent' + dirSuffix] = packets

		circuit['timeQueried'] = datetime.now().isoformat()
	for circuit in subscriberCircuits:
		circuit['bitsDownloadSinceLastQuery'] = circuit['bitsUploadSinceLastQuery'] = 0
		circuit['packetDropsDownloadSinceLastQuery'] = circuit['packetDropsUploadSinceLastQuery'] = 0
		if 'priorQueryTime' in circuit:
			try:
				bytesDLSinceLastQuery = circuit['bytesSentDownload'] - circuit['priorQueryBytesDownload']
				bytesULSinceLastQuery = circuit['bytesSentUpload'] - circuit['priorQueryBytesUpload']
			except:
				bytesDLSinceLastQuery = bytesULSinceLastQuery = 0
			try:
				packetDropsDLSinceLastQuery = circuit['dropsSentDownload'] - circuit['priorQueryDropsDownload']
				packetDropsULSinceLastQuery = circuit['dropsSentUpload'] - circuit['priorQueryDropsUpload']
			except:
				packetDropsDLSinceLastQuery = packetDropsULSinceLastQuery = 0
			currentQueryTime = datetime.fromisoformat(circuit['timeQueried'])
			priorQueryTime = datetime.fromisoformat(circuit['priorQueryTime'])
			deltaSeconds = (currentQueryTime - priorQueryTime).total_seconds()

			circuit['bitsDownloadSinceLastQuery'] = round(
				((bytesDLSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0
			circuit['bitsUploadSinceLastQuery'] = round(
				((bytesULSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0
			circuit['packetDropsDownloadSinceLastQuery'] = packetDropsDLSinceLastQuery
			circuit['packetDropsUploadSinceLastQuery'] = packetDropsULSinceLastQuery

	return subscriberCircuits


def getParentNodeStats(parentNodes, subscriberCircuits):
	for parentNode in parentNodes:
		thisNodeDropsDownload = 0
		thisNodeDropsUpload = 0
		thisNodeDropsTotal = 0
		thisNodeBitsDownload = 0
		thisNodeBitsUpload = 0
		for circuit in subscriberCircuits:
			if circuit['ParentNode'] == parentNode['parentNodeName']:
				thisNodeBitsDownload += circuit['bitsDownloadSinceLastQuery']
				thisNodeBitsUpload += circuit['bitsUploadSinceLastQuery']
				thisNodeDropsDownload += circuit['packetDropsDownloadSinceLastQuery']
				thisNodeDropsUpload += circuit['packetDropsUploadSinceLastQuery']
				thisNodeDropsTotal += (circuit['packetDropsDownloadSinceLastQuery'] + circuit['packetDropsUploadSinceLastQuery'])
		parentNode['bitsDownloadSinceLastQuery'] = thisNodeBitsDownload
		parentNode['bitsUploadSinceLastQuery'] = thisNodeBitsUpload
		parentNode['packetDropsDownloadSinceLastQuery'] = thisNodeDropsDownload
		parentNode['packetDropsUploadSinceLastQuery'] = thisNodeDropsUpload
		parentNode['packetDropsTotalSinceLastQuery'] = thisNodeDropsTotal
	return parentNodes


def getParentNodeDict(data, depth, parentNodeNameDict):
	if parentNodeNameDict == None:
		parentNodeNameDict = {}

	for elem in data:
		if 'children' in data[elem]:
			for child in data[elem]['children']:
				parentNodeNameDict[child] = elem
			tempDict = getParentNodeDict(data[elem]['children'], depth + 1, parentNodeNameDict)
			parentNodeNameDict = dict(parentNodeNameDict, **tempDict)
	return parentNodeNameDict


def parentNodeNameDictPull():
	# Load network heirarchy
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
	subscriberCircuits = getsubscriberCircuitstats(subscriberCircuits)
	print("Computing parent node statistics")
	parentNodes = getParentNodeStats(parentNodes, subscriberCircuits)
	print("Writing data to InfluxDB")
	client = InfluxDBClient(
		url=influxDBurl,
		token=influxDBtoken,
		org=influxDBOrg
	)
	write_api = client.write_api(write_options=SYNCHRONOUS)

	chunkedsubscriberCircuits = list(chunk_list(subscriberCircuits, 200))

	queriesToSendCount = 0
	for chunk in chunkedsubscriberCircuits:
		queriesToSend = []
		for circuit in chunk:
			bitsDownload = float(circuit['bitsDownloadSinceLastQuery'])
			bitsUpload = float(circuit['bitsUploadSinceLastQuery'])
			if (bitsDownload > 0) and (bitsUpload > 0):
				percentUtilizationDownload = round((bitsDownload / round(circuit['downloadMax'] * 1000000)), 2)
				percentUtilizationUpload = round((bitsUpload / round(circuit['uploadMax'] * 1000000)), 2)
				p = Point('Bandwidth').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", bitsDownload).field("Upload", bitsUpload)
				queriesToSend.append(p)
				p = Point('Utilization').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload)
				queriesToSend.append(p)

		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	for parentNode in parentNodes:
		bitsDownload = float(parentNode['bitsDownloadSinceLastQuery'])
		bitsUpload = float(parentNode['bitsUploadSinceLastQuery'])
		dropsTotal = float(parentNode['packetDropsTotalSinceLastQuery'])
		if (bitsDownload > 0) and (bitsUpload > 0):
			percentUtilizationDownload = round((bitsDownload / round(parentNode['downloadMax'] * 1000000)), 2)
			percentUtilizationUpload = round((bitsUpload / round(parentNode['uploadMax'] * 1000000)), 2)
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload).field("Upload", bitsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)
			p = Point('Drops').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Drops", dropsTotal)
			queriesToSend.append(p)

	write_api.write(bucket=influxDBBucket, record=queriesToSend)
	# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	queriesToSendCount += len(queriesToSend)
	print("Added " + str(queriesToSendCount) + " points to InfluxDB.")

	client.close()

	with open('statsByParentNode.json', 'w') as infile:
		json.dump(parentNodes, infile)

	with open('statsByCircuit.json', 'w') as infile:
		json.dump(subscriberCircuits, infile)

	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(), 2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")

if __name__ == '__main__':
	refreshBandwidthGraphs()
