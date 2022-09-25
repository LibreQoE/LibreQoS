import subprocess
import json
import subprocess
from datetime import datetime
from pathlib import Path

from influxdb_client import InfluxDBClient, Point
from influxdb_client.client.write_api import SYNCHRONOUS

from ispConfig import interfaceA, interfaceB, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl, fqOrCAKE


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
	tinsBulkPacketsSentDownload = tinsBestEffortPacketsSentDownload = tinsVoicePacketsSentDownload =tinsVideoPacketsSentDownload = 0.0
	tinsBulkPacketsSentUpload = tinsBestEffortPacketsSentUpload = tinsVoicePacketsSentUpload =tinsVideoPacketsSentUpload = 0.0
	allPacketsDownload = 0.0
	allPacketsUpload = 0.0
	
	for circuit in subscriberCircuits:
		if 'timeQueried' in circuit:
			circuit['priorQueryTime'] = circuit['timeQueried']
		for (interface, stats, dirSuffix) in zip(interfaces, ifaceStats, ['Download', 'Upload']):

			element = stats[circuit['qdisc']] if circuit['qdisc'] in stats else False

			if element:

				bytesSent = float(element['bytes'])
				drops = float(element['drops'])
				packets = float(element['packets'])
				if (element['drops'] > 0) and (element['packets'] > 0):
					overloadFactor = float(round(element['drops']/element['packets'],3))
				else:
					overloadFactor = 0.0
				
				if 'cake diffserv4' in fqOrCAKE:
					tinCounter = 1
					packetsSentBulk = 0.0
					packetsSentBestEffort = 0.0
					packetsSentVideo = 0.0
					packetsSentVoice = 0.0
					for tin in element['tins']:
						sent_packets = float(tin['sent_packets'])
						ack_drops = float(tin['ack_drops'])
						ecn_mark = float(tin['ecn_mark'])
						tinDrops = float(tin['drops'])
						trueDrops = tinDrops - ack_drops
						if tinCounter == 1:
							packetsSentBulk = sent_packets
						elif tinCounter == 2:
							packetsSentBestEffort = sent_packets
						elif tinCounter == 3:
							packetsSentVideo = sent_packets
						elif tinCounter == 4:
							packetsSentVoice = sent_packets
						tinCounter += 1

				if 'bytesSent' + dirSuffix in circuit:
					circuit['priorQueryBytes' + dirSuffix] = circuit['bytesSent' + dirSuffix]
				circuit['bytesSent' + dirSuffix] = bytesSent

				if 'dropsSent' + dirSuffix in circuit:
					circuit['priorQueryDrops' + dirSuffix] = circuit['dropsSent' + dirSuffix]
				circuit['dropsSent' + dirSuffix] = drops

				if 'packetsSent' + dirSuffix in circuit:
					circuit['priorQueryPacketsSent' + dirSuffix] = circuit['packetsSent' + dirSuffix]
				circuit['packetsSent' + dirSuffix] = packets
				
				if 'overloadFactor' + dirSuffix in circuit:
					circuit['priorQueryOverloadFactor' + dirSuffix] = circuit['overloadFactor' + dirSuffix]
				circuit['overloadFactor' + dirSuffix] = overloadFactor
				
				if 'cake diffserv4' in fqOrCAKE:
					if 'packetsSentBulk' + dirSuffix in circuit:
						circuit['priorQueryPacketsSentBulk' + dirSuffix] = circuit['packetsSentBulk' + dirSuffix]
					circuit['packetsSentBulk' + dirSuffix] = packetsSentBulk
					
					if 'packetsSentBestEffort' + dirSuffix in circuit:
						circuit['priorQueryPacketsSentBestEffort' + dirSuffix] = circuit['packetsSentBestEffort' + dirSuffix]
					circuit['packetsSentBestEffort' + dirSuffix] = packetsSentBestEffort
					
					if 'packetsSentVideo' + dirSuffix in circuit:
						circuit['priorQueryPacketsSentVideo' + dirSuffix] = circuit['packetsSentVideo' + dirSuffix]
					circuit['packetsSentVideo' + dirSuffix] = packetsSentVideo
					
					if 'packetsSentVoice' + dirSuffix in circuit:
						circuit['priorQueryPacketsSentVoice' + dirSuffix] = circuit['packetsSentVoice' + dirSuffix]
					circuit['packetsSentVoice' + dirSuffix] = packetsSentVoice

		circuit['timeQueried'] = datetime.now().isoformat()
	for circuit in subscriberCircuits:
		circuit['bitsDownloadSinceLastQuery'] = circuit['bitsUploadSinceLastQuery'] = 0.0
		circuit['packetDropsDownloadSinceLastQuery'] = circuit['packetDropsUploadSinceLastQuery'] = 0.0
		circuit['packetsSentDownloadSinceLastQuery'] = circuit['packetsSentUploadSinceLastQuery'] = 0.0
		if 'priorQueryTime' in circuit:
			try:
				bytesDLSinceLastQuery = circuit['bytesSentDownload'] - circuit['priorQueryBytesDownload']
				bytesULSinceLastQuery = circuit['bytesSentUpload'] - circuit['priorQueryBytesUpload']
			except:
				bytesDLSinceLastQuery = bytesULSinceLastQuery = 0.0
			try:
				packetDropsDLSinceLastQuery = circuit['dropsSentDownload'] - circuit['priorQueryDropsDownload']
				packetDropsULSinceLastQuery = circuit['dropsSentUpload'] - circuit['priorQueryDropsUpload']
			except:
				packetDropsDLSinceLastQuery = packetDropsULSinceLastQuery = 0.0
			try:
				packetsSentDLSinceLastQuery = circuit['packetsSentDownload'] - circuit['priorQueryPacketsSentDownload']
				packetsSentULSinceLastQuery = circuit['packetsSentUpload'] - circuit['priorQueryPacketsSentUpload']
			except:
				packetsSentDLSinceLastQuery = packetsSentULSinceLastQuery = 0.0
			
			if 'cake diffserv4' in fqOrCAKE:
				try:
					packetsSentDLSinceLastQueryBulk = circuit['packetsSentBulkDownload'] - circuit['priorQueryPacketsSentBulkDownload']
					packetsSentULSinceLastQueryBulk = circuit['packetsSentBulkUpload'] - circuit['priorQueryPacketsSentBulkUpload']
					packetsSentDLSinceLastQueryBestEffort = circuit['packetsSentBestEffortDownload'] - circuit['priorQueryPacketsSentBestEffortDownload']
					packetsSentULSinceLastQueryBestEffort = circuit['packetsSentBestEffortUpload'] - circuit['priorQueryPacketsSentBestEffortUpload']
					packetsSentDLSinceLastQueryVideo = circuit['packetsSentVideoDownload'] - circuit['priorQueryPacketsSentVideoDownload']
					packetsSentULSinceLastQueryVideo = circuit['packetsSentVideoUpload'] - circuit['priorQueryPacketsSentVideoUpload']
					packetsSentDLSinceLastQueryVoice = circuit['packetsSentVoiceDownload'] - circuit['priorQueryPacketsSentVoiceDownload']
					packetsSentULSinceLastQueryVoice = circuit['packetsSentVoiceUpload'] - circuit['priorQueryPacketsSentVoiceUpload']
				except:
					packetsSentDLSinceLastQueryBulk = packetsSentULSinceLastQueryBulk = 0.0
					packetsSentDLSinceLastQueryBestEffort = packetsSentULSinceLastQueryBestEffort = 0.0
					packetsSentDLSinceLastQueryVideo = packetsSentULSinceLastQueryVideo = 0.0
					packetsSentDLSinceLastQueryVoice = packetsSentULSinceLastQueryVoice = 0.0
				
				allPacketsDownload += packetsSentDLSinceLastQuery
				allPacketsUpload += packetsSentULSinceLastQuery
				
				tinsBulkPacketsSentDownload += packetsSentDLSinceLastQueryBulk
				tinsBestEffortPacketsSentDownload += packetsSentDLSinceLastQueryBestEffort
				tinsVideoPacketsSentDownload += packetsSentDLSinceLastQueryVideo
				tinsVoicePacketsSentDownload += packetsSentDLSinceLastQueryVoice
				tinsBulkPacketsSentUpload += packetsSentULSinceLastQueryBulk
				tinsBestEffortPacketsSentUpload += packetsSentULSinceLastQueryBestEffort
				tinsVideoPacketsSentUpload += packetsSentULSinceLastQueryVideo
				tinsVoicePacketsSentUpload += packetsSentULSinceLastQueryVoice
						
			currentQueryTime = datetime.fromisoformat(circuit['timeQueried'])
			priorQueryTime = datetime.fromisoformat(circuit['priorQueryTime'])
			deltaSeconds = (currentQueryTime - priorQueryTime).total_seconds()

			circuit['bitsDownloadSinceLastQuery'] = round(
				((bytesDLSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0
			circuit['bitsUploadSinceLastQuery'] = round(
				((bytesULSinceLastQuery * 8) / deltaSeconds)) if deltaSeconds > 0 else 0
			circuit['packetDropsDownloadSinceLastQuery'] = packetDropsDLSinceLastQuery
			circuit['packetDropsUploadSinceLastQuery'] = packetDropsULSinceLastQuery
			circuit['packetsSentDownloadSinceLastQuery'] = packetsSentDLSinceLastQuery
			circuit['packetsSentUploadSinceLastQuery'] = packetsSentULSinceLastQuery
	
	if 'cake diffserv4' in fqOrCAKE:
		tinsStats = {	'Bulk': {}, 
						'BestEffort': {},
						'Voice': {},
						'Video': {}}
		
		if (allPacketsDownload > 0):
			tinsStats['Bulk']['Download'] = round((tinsBulkPacketsSentDownload / allPacketsDownload) * 100.0, 1)
			tinsStats['BestEffort']['Download'] = round((tinsBestEffortPacketsSentDownload / allPacketsDownload) * 100.0, 1)
			tinsStats['Voice']['Download'] =  round((tinsVoicePacketsSentDownload / allPacketsDownload) * 100.0, 1)
			tinsStats['Video']['Download'] =  round((tinsVideoPacketsSentDownload / allPacketsDownload) * 100.0, 1)
		else:
			tinsStats['Bulk']['Download'] = 0.0
			tinsStats['BestEffort']['Download'] = 0.0
			tinsStats['Voice']['Download'] =  0.0
			tinsStats['Video']['Download'] =  0.0
		if (allPacketsUpload > 0):
			tinsStats['Bulk']['Upload'] = round((tinsBulkPacketsSentUpload / allPacketsUpload) * 100.0, 1)
			tinsStats['BestEffort']['Upload'] =  round((tinsBestEffortPacketsSentUpload / allPacketsUpload) * 100.0, 1)
			tinsStats['Video']['Upload'] =  round((tinsVideoPacketsSentUpload / allPacketsUpload) * 100.0, 1)
			tinsStats['Voice']['Upload'] =  round((tinsVoicePacketsSentUpload / allPacketsUpload) * 100.0, 1)
		else:
			tinsStats['Bulk']['Upload'] = 0.0
			tinsStats['BestEffort']['Upload'] =  0.0
			tinsStats['Video']['Upload'] =  0.0
			tinsStats['Voice']['Upload'] =  0.0
	else:
		tinsStats = {	'Bulk': {}, 
						'BestEffort': {},
						'Voice': {},
						'Video': {}}
		tinsStats['Bulk']['Download'] = 0.0
		tinsStats['BestEffort']['Download'] = 0.0
		tinsStats['Voice']['Download'] =  0.0
		tinsStats['Video']['Download'] =  0.0
		tinsStats['Bulk']['Upload'] = 0.0
		tinsStats['BestEffort']['Upload'] = 0.0
		tinsStats['Voice']['Upload'] =  0.0
		tinsStats['Video']['Upload'] =  0.0

	return subscriberCircuits, tinsStats


def getParentNodeStats(parentNodes, subscriberCircuits):
	for parentNode in parentNodes:
		thisNodeDropsDownload = 0
		thisNodeDropsUpload = 0
		thisNodeDropsTotal = 0
		thisNodeBitsDownload = 0
		thisNodeBitsUpload = 0
		packetsSentDownloadAggregate = 0.0
		packetsSentUploadAggregate = 0.0
		packetsSentTotalAggregate = 0.0
		circuitsMatched = 0
		for circuit in subscriberCircuits:
			if circuit['ParentNode'] == parentNode['parentNodeName']:
				thisNodeBitsDownload += circuit['bitsDownloadSinceLastQuery']
				thisNodeBitsUpload += circuit['bitsUploadSinceLastQuery']
				#thisNodeDropsDownload += circuit['packetDropsDownloadSinceLastQuery']
				#thisNodeDropsUpload += circuit['packetDropsUploadSinceLastQuery']
				thisNodeDropsTotal += (circuit['packetDropsDownloadSinceLastQuery'] + circuit['packetDropsUploadSinceLastQuery'])
				packetsSentDownloadAggregate += circuit['packetsSentDownloadSinceLastQuery']
				packetsSentUploadAggregate += circuit['packetsSentUploadSinceLastQuery']
				packetsSentTotalAggregate += (circuit['packetsSentDownloadSinceLastQuery'] + circuit['packetsSentUploadSinceLastQuery'])
				circuitsMatched += 1
		if (packetsSentDownloadAggregate > 0) and (packetsSentUploadAggregate > 0):
			#overloadFactorDownloadSinceLastQuery = float(round((thisNodeDropsDownload/packetsSentDownloadAggregate)*100.0, 3))
			#overloadFactorUploadSinceLastQuery = float(round((thisNodeDropsUpload/packetsSentUploadAggregate)*100.0, 3))
			overloadFactorTotalSinceLastQuery = float(round((thisNodeDropsTotal/packetsSentTotalAggregate)*100.0, 1))
		else:
			#overloadFactorDownloadSinceLastQuery = 0.0
			#overloadFactorUploadSinceLastQuery = 0.0
			overloadFactorTotalSinceLastQuery = 0.0
		
		parentNode['bitsDownloadSinceLastQuery'] = thisNodeBitsDownload
		parentNode['bitsUploadSinceLastQuery'] = thisNodeBitsUpload
		#parentNode['packetDropsDownloadSinceLastQuery'] = thisNodeDropsDownload
		#parentNode['packetDropsUploadSinceLastQuery'] = thisNodeDropsUpload
		parentNode['packetDropsTotalSinceLastQuery'] = thisNodeDropsTotal
		#parentNode['overloadFactorDownloadSinceLastQuery'] = overloadFactorDownloadSinceLastQuery
		#parentNode['overloadFactorUploadSinceLastQuery'] = overloadFactorUploadSinceLastQuery
		parentNode['overloadFactorTotalSinceLastQuery'] = overloadFactorTotalSinceLastQuery
		
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
	
	fileLoc = Path("longTermStats.json")
	if fileLoc.is_file():
		with open(fileLoc, 'r') as j:
			longTermStats = json.loads(j.read())
		droppedPacketsAllTime = longTermStats['droppedPacketsTotal']
	else:
		longTermStats = {}
		longTermStats['droppedPacketsTotal'] = 0.0
		droppedPacketsAllTime = 0.0

	parentNodeNameDict = parentNodeNameDictPull()

	print("Retrieving circuit statistics")
	subscriberCircuits, tinsStats = getsubscriberCircuitstats(subscriberCircuits)
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
				percentUtilizationDownload = round((bitsDownload / round(circuit['downloadMax'] * 1000000))*100.0, 1)
				percentUtilizationUpload = round((bitsUpload / round(circuit['uploadMax'] * 1000000))*100.0, 1)
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
		overloadFactor = float(parentNode['overloadFactorTotalSinceLastQuery'])
		droppedPacketsAllTime += dropsTotal
		if (bitsDownload > 0) and (bitsUpload > 0):
			percentUtilizationDownload = round((bitsDownload / round(parentNode['downloadMax'] * 1000000))*100.0, 1)
			percentUtilizationUpload = round((bitsUpload / round(parentNode['uploadMax'] * 1000000))*100.0, 1)
			p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload).field("Upload", bitsUpload)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload)
			queriesToSend.append(p)
			p = Point('Overload').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Overload", overloadFactor)
			queriesToSend.append(p)

	write_api.write(bucket=influxDBBucket, record=queriesToSend)
	# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	queriesToSendCount += len(queriesToSend)
	
	if 'cake diffserv4' in fqOrCAKE:
		queriesToSend = []
		for tin in tinsStats:
			tinName = tin
			p = Point('Tins').tag("Type", "Tin").tag("Tin", tinName).field("Download", tinsStats[tin]['Download']).field("Upload", tinsStats[tin]['Upload'])
			queriesToSend.append(p)

		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)
	
	print("Added " + str(queriesToSendCount) + " points to InfluxDB.")

	client.close()
	
	with open('statsByParentNode.json', 'w') as f:
		f.write(json.dumps(parentNodes, indent=4))

	with open('statsByCircuit.json', 'w') as f:
		f.write(json.dumps(subscriberCircuits, indent=4))
	
	longTermStats['droppedPacketsTotal'] = droppedPacketsAllTime
	with open('longTermStats.json', 'w') as f:
		f.write(json.dumps(longTermStats, indent=4))

	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(), 2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")

if __name__ == '__main__':
	refreshBandwidthGraphs()
