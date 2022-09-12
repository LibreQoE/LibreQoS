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


def getDeviceStats(devices):
	interfaces = [interfaceA, interfaceB]
	ifaceStats = list(map(getInterfaceStats, interfaces))

	for device in devices:
		if 'timeQueried' in device:
			device['priorQueryTime'] = device['timeQueried']
		for (interface, stats, dirSuffix) in zip(interfaces, ifaceStats, ['Download', 'Upload']):

			element = stats[device['qdisc']] if device['qdisc'] in stats else False

			if element:

				bytesSent = int(element['bytes'])
				drops = int(element['drops'])
				packets = int(element['packets'])

				if 'bytesSent' + dirSuffix in device:
					device['priorQueryBytes' + dirSuffix] = device['bytesSent' + dirSuffix]
				device['bytesSent' + dirSuffix] = bytesSent

				if 'dropsSent' + dirSuffix in device:
					device['priorDropsSent' + dirSuffix] = device['dropsSent' + dirSuffix]
				device['dropsSent' + dirSuffix] = drops

				if 'packetsSent' + dirSuffix in device:
					device['priorPacketsSent' + dirSuffix] = device['packetsSent' + dirSuffix]
				device['packetsSent' + dirSuffix] = packets

		device['timeQueried'] = datetime.now().isoformat()
	for device in devices:
		device['bitsDownloadSinceLastQuery'] = device['bitsUploadSinceLastQuery'] = 0
		if 'priorQueryTime' in device:
			try:
				bytesDLSinceLastQuery = device['bytesSentDownload'] - device['priorQueryBytesDownload']
				bytesULSinceLastQuery = device['bytesSentUpload'] - device['priorQueryBytesUpload']
			except:
				bytesDLSinceLastQuery = bytesULSinceLastQuery = 0

			currentQueryTime = datetime.fromisoformat(device['timeQueried'])
			priorQueryTime = datetime.fromisoformat(device['priorQueryTime'])
			deltaSeconds = (currentQueryTime - priorQueryTime).total_seconds()

			device['bitsDownloadSinceLastQuery'] = round(
				((bytesDLSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0
			device['bitsUploadSinceLastQuery'] = round(
				((bytesULSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0

	return devices


def getParentNodeStats(parentNodes, devices):
	for parentNode in parentNodes:
		thisNodeBitsDownload = 0
		thisNodeBitsUpload = 0
		for device in devices:
			if device['ParentNode'] == parentNode['parentNodeName']:
				thisNodeBitsDownload += device['bitsDownloadSinceLastQuery']
				thisNodeBitsUpload += device['bitsUploadSinceLastQuery']

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

	with open('statsByDevice.json', 'r') as j:
		devices = json.loads(j.read())

	parentNodeNameDict = parentNodeNameDictPull()

	print("Retrieving device statistics")
	devices = getDeviceStats(devices)
	print("Computing parent node statistics")
	parentNodes = getParentNodeStats(parentNodes, devices)
	print("Writing data to InfluxDB")
	client = InfluxDBClient(
		url=influxDBurl,
		token=influxDBtoken,
		org=influxDBOrg
	)
	write_api = client.write_api(write_options=SYNCHRONOUS)

	chunkedDevices = list(chunk_list(devices, 200))

	queriesToSendCount = 0
	for chunk in chunkedDevices:
		queriesToSend = []
		for device in chunk:
			bitsDownload = int(device['bitsDownloadSinceLastQuery'])
			bitsUpload = int(device['bitsUploadSinceLastQuery'])
			if (bitsDownload > 0) and (bitsUpload > 0):
				percentUtilizationDownload = round((bitsDownload / round(device['downloadMax'] * 1000000)), 4)
				percentUtilizationUpload = round((bitsUpload / round(device['uploadMax'] * 1000000)), 4)
				p = Point('Bandwidth').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).tag("Type", "Circuit").field("Download", bitsDownload).field("Upload", bitsUpload)
				queriesToSend.append(p)
				p = Point('Utilization').tag("Device", device['hostname']).tag("ParentNode", device['ParentNode']).tag("Type", "Circuit").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload)
				queriesToSend.append(p)

		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	for parentNode in parentNodes:
		bitsDownload = int(parentNode['bitsDownloadSinceLastQuery'])
		bitsUpload = int(parentNode['bitsUploadSinceLastQuery'])
		if (bitsDownload > 0) and (bitsUpload > 0):
			percentUtilizationDownload = round((bitsDownload / round(parentNode['downloadMax'] * 1000000)), 4)
			percentUtilizationUpload = round((bitsUpload / round(parentNode['uploadMax'] * 1000000)), 4)
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload).field("Upload", bitsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)

	write_api.write(bucket=influxDBBucket, record=queriesToSend)
	# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	queriesToSendCount += len(queriesToSend)
	print("Added " + str(queriesToSendCount) + " points to InfluxDB.")

	client.close()


	with open('statsByParentNode.json', 'w') as infile:
		json.dump(parentNodes, infile)

	with open('statsByDevice.json', 'w') as infile:
		json.dump(devices, infile)

	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(), 2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")

if __name__ == '__main__':
	refreshBandwidthGraphs()
