import subprocess
import json
import subprocess
from datetime import datetime
from pathlib import Path
import statistics
import time
import psutil
from pprint import pprint

from influxdb_client import InfluxDBClient, Point
from influxdb_client.client.write_api import SYNCHRONOUS

from ispConfig import interfaceA, interfaceB, influxDBEnabled, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl, sqm

class exceptionWithMessage(Exception):
	def __init__(self, message, detail = None):
		self.message = message
		trace = "not able to retrieve trace"
		trace = traceback.format_exc()
		notifySpike(message, trace, detail)
		super().__init__(self.message)

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

def getCircuitBandwidthStats(subscriberCircuits, tinsStats):
	
	try:
		interfaces = [interfaceA, interfaceB]
		ifaceStats = list(map(getInterfaceStats, interfaces))
		
		for circuit in subscriberCircuits:
			if 'stats' not in circuit:
				circuit['stats'] = {}
			if 'currentQuery' in circuit['stats']:
				circuit['stats']['priorQuery'] = circuit['stats']['currentQuery']
				circuit['stats']['currentQuery'] = {}
				circuit['stats']['sinceLastQuery'] = {}
			else:
				#circuit['stats']['priorQuery'] = {}
				#circuit['stats']['priorQuery']['time'] = datetime.now().isoformat()
				circuit['stats']['currentQuery'] = {}
				circuit['stats']['sinceLastQuery'] = {}
			if 'tinsStats' not in circuit:
				circuit['tinsStats'] = {}
			if 'currentQuery' in circuit['tinsStats']:
				circuit['tinsStats']['priorQuery'] = circuit['tinsStats']['currentQuery']
				circuit['tinsStats']['currentQuery'] = {}
				circuit['tinsStats']['sinceLastQuery'] = {}
			else:
				circuit['tinsStats']['currentQuery'] = {}
				circuit['tinsStats']['sinceLastQuery'] = {}

		#for entry in tinsStats:
		if 'currentQuery' in tinsStats:
			tinsStats['priorQuery'] = tinsStats['currentQuery']
			tinsStats['currentQuery'] = {}
			tinsStats['sinceLastQuery'] = {}
		else:
			tinsStats['currentQuery'] = {}
			tinsStats['sinceLastQuery'] = {}
		
		tinsStats['currentQuery'] = {	'Bulk': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'BestEffort': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'Video': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'Voice': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
									}
		tinsStats['sinceLastQuery'] = {	'Bulk': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'BestEffort': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'Video': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
										'Voice': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
									}
		
		for circuit in subscriberCircuits:
			circuit['tinsStats']['currentQuery'] = { 'Bulk': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'BestEffort': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'Video': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'Voice': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
			}
			circuit['tinsStats']['sinceLastQuery'] = { 'Bulk': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'BestEffort': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'Video': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
				'Voice': {'Download': {'sent_packets': 0.0, 'drops': 0.0}, 'Upload': {'sent_packets': 0.0, 'drops': 0.0}},
			}
			for (interface, stats, dirSuffix) in zip(interfaces, ifaceStats, ['Download', 'Upload']):

				element = stats[circuit['classid']] if circuit['classid'] in stats else False

				if element:
					bytesSent = float(element['bytes'])

					""" if "priorQuery" in circuit['stats']:
						if bytesSent - circuit['stats']['priorQuery']['bytesSentDownload'] < 0.0:
							exceptionWithMessage(
								"Less download data usage now versus the past, this should not be possible.",
								{
									"circuit": json.dumps(circuit),
									"element": json.dumps(element)
								}
							) """

					drops = float(element['drops'])
					packets = float(element['packets'])
					if (element['drops'] > 0) and (element['packets'] > 0):
						overloadFactor = float(round(element['drops']/element['packets'],3))
					else:
						overloadFactor = 0.0
					
					if 'cake diffserv4' in sqm:
						tinCounter = 1
						for tin in element['tins']:
							sent_packets = float(tin['sent_packets'])
							ack_drops = float(tin['ack_drops'])
							ecn_mark = float(tin['ecn_mark'])
							tinDrops = float(tin['drops'])
							trueDrops = ecn_mark + tinDrops - ack_drops
							if tinCounter == 1:
								tinsStats['currentQuery']['Bulk'][dirSuffix]['sent_packets'] += sent_packets
								circuit['tinsStats']['currentQuery']['Bulk'][dirSuffix]['sent_packets'] += sent_packets
								tinsStats['currentQuery']['Bulk'][dirSuffix]['drops'] += trueDrops
								circuit['tinsStats']['currentQuery']['Bulk'][dirSuffix]['drops'] += trueDrops
							elif tinCounter == 2:
								tinsStats['currentQuery']['BestEffort'][dirSuffix]['sent_packets'] += sent_packets
								circuit['tinsStats']['currentQuery']['BestEffort'][dirSuffix]['sent_packets'] += sent_packets
								tinsStats['currentQuery']['BestEffort'][dirSuffix]['drops'] += trueDrops
								circuit['tinsStats']['currentQuery']['BestEffort'][dirSuffix]['drops'] += trueDrops
							elif tinCounter == 3:
								tinsStats['currentQuery']['Video'][dirSuffix]['sent_packets'] += sent_packets
								circuit['tinsStats']['currentQuery']['Video'][dirSuffix]['sent_packets'] += sent_packets
								tinsStats['currentQuery']['Video'][dirSuffix]['drops'] += trueDrops
								circuit['tinsStats']['currentQuery']['Video'][dirSuffix]['drops'] += trueDrops
							elif tinCounter == 4:
								tinsStats['currentQuery']['Voice'][dirSuffix]['sent_packets'] += sent_packets
								circuit['tinsStats']['currentQuery']['Voice'][dirSuffix]['sent_packets'] += sent_packets
								tinsStats['currentQuery']['Voice'][dirSuffix]['drops'] += trueDrops
								circuit['tinsStats']['currentQuery']['Voice'][dirSuffix]['drops'] += trueDrops
							tinCounter += 1

					circuit['stats']['currentQuery']['bytesSent' + dirSuffix] = bytesSent
					circuit['stats']['currentQuery']['packetDrops' + dirSuffix] = drops
					circuit['stats']['currentQuery']['packetsSent' + dirSuffix] = packets
					circuit['stats']['currentQuery']['overloadFactor' + dirSuffix] = overloadFactor
					
					#if 'cake diffserv4' in sqm:
					#	circuit['stats']['currentQuery']['tins'] = theseTins

			circuit['stats']['currentQuery']['time'] = datetime.now().isoformat()
			
		allPacketsDownload = 0.0
		allPacketsUpload = 0.0
		for circuit in subscriberCircuits:
			circuit['stats']['sinceLastQuery']['bitsDownload'] = circuit['stats']['sinceLastQuery']['bitsUpload'] = 0.0
			circuit['stats']['sinceLastQuery']['bytesSentDownload'] = circuit['stats']['sinceLastQuery']['bytesSentUpload'] = 0.0
			circuit['stats']['sinceLastQuery']['packetDropsDownload'] = circuit['stats']['sinceLastQuery']['packetDropsUpload'] = 0.0
			circuit['stats']['sinceLastQuery']['packetsSentDownload'] = circuit['stats']['sinceLastQuery']['packetsSentUpload'] = 0.0
			
			try:
				if (circuit['stats']['currentQuery']['bytesSentDownload'] - circuit['stats']['priorQuery']['bytesSentDownload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['bytesSentDownload'] = circuit['stats']['currentQuery']['bytesSentDownload'] - circuit['stats']['priorQuery']['bytesSentDownload']
				else:
					circuit['stats']['sinceLastQuery']['bytesSentDownload'] = 0.0
				if (circuit['stats']['currentQuery']['bytesSentUpload'] - circuit['stats']['priorQuery']['bytesSentUpload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['bytesSentUpload'] = circuit['stats']['currentQuery']['bytesSentUpload'] - circuit['stats']['priorQuery']['bytesSentUpload']
				else:
					circuit['stats']['sinceLastQuery']['bytesSentUpload'] = 0.0
			except:
				circuit['stats']['sinceLastQuery']['bytesSentDownload'] = 0.0
				circuit['stats']['sinceLastQuery']['bytesSentUpload'] = 0.0
			try:
				if (circuit['stats']['currentQuery']['packetDropsDownload'] - circuit['stats']['priorQuery']['packetDropsDownload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['packetDropsDownload'] = circuit['stats']['currentQuery']['packetDropsDownload'] - circuit['stats']['priorQuery']['packetDropsDownload']
				else:
					circuit['stats']['sinceLastQuery']['packetDropsDownload'] = 0.0
				if (circuit['stats']['currentQuery']['packetDropsUpload'] - circuit['stats']['priorQuery']['packetDropsUpload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['packetDropsUpload'] = circuit['stats']['currentQuery']['packetDropsUpload'] - circuit['stats']['priorQuery']['packetDropsUpload']
				else:
					circuit['stats']['sinceLastQuery']['packetDropsUpload'] = 0.0
			except:
				circuit['stats']['sinceLastQuery']['packetDropsDownload'] = 0.0
				circuit['stats']['sinceLastQuery']['packetDropsUpload'] = 0.0
			try:
				if (circuit['stats']['currentQuery']['packetsSentDownload'] - circuit['stats']['priorQuery']['packetsSentDownload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['packetsSentDownload'] = circuit['stats']['currentQuery']['packetsSentDownload'] - circuit['stats']['priorQuery']['packetsSentDownload']
				else:
					circuit['stats']['sinceLastQuery']['packetsSentDownload'] = 0.0
				if (circuit['stats']['currentQuery']['packetsSentUpload'] - circuit['stats']['priorQuery']['packetsSentUpload']) >= 0.0:
					circuit['stats']['sinceLastQuery']['packetsSentUpload'] = circuit['stats']['currentQuery']['packetsSentUpload'] - circuit['stats']['priorQuery']['packetsSentUpload']
				else:
					circuit['stats']['sinceLastQuery']['packetsSentUpload'] = 0.0
			except:
				circuit['stats']['sinceLastQuery']['packetsSentDownload'] = 0.0
				circuit['stats']['sinceLastQuery']['packetsSentUpload'] = 0.0
			
			allPacketsDownload += circuit['stats']['sinceLastQuery']['packetsSentDownload']
			allPacketsUpload += circuit['stats']['sinceLastQuery']['packetsSentUpload']
			
			if 'priorQuery' in circuit['stats']:
				if 'time' in circuit['stats']['priorQuery']:
					currentQueryTime = datetime.fromisoformat(circuit['stats']['currentQuery']['time'])
					priorQueryTime = datetime.fromisoformat(circuit['stats']['priorQuery']['time'])
					deltaSeconds = (currentQueryTime - priorQueryTime).total_seconds()
					if deltaSeconds < 0:
						exceptionWithMessage("Current query time ({}) is before last query time ({}), time sync may be broken. serviceId is {}. Name is: {}.".format(currentQueryTime, priorQueryTime, circuit['circuitID'], circuit['circuitName']))
					circuit['stats']['sinceLastQuery']['bitsDownload'] = round((circuit['stats']['sinceLastQuery']['bytesSentDownload'] * 8) / deltaSeconds) if deltaSeconds > 0 else 0
					circuit['stats']['sinceLastQuery']['bitsUpload'] = round((circuit['stats']['sinceLastQuery']['bytesSentUpload'] * 8) / deltaSeconds) if deltaSeconds > 0 else 0
			else:
				circuit['stats']['sinceLastQuery']['bitsDownload'] = (circuit['stats']['sinceLastQuery']['bytesSentDownload'] * 8)
				circuit['stats']['sinceLastQuery']['bitsUpload'] = (circuit['stats']['sinceLastQuery']['bytesSentUpload'] * 8)

			# Process Tin Statistics by Circuit
			circuit['tinsStats'] = buildTinStats(circuit['tinsStats'], circuit['stats']['sinceLastQuery']['packetsSentDownload'], circuit['stats']['sinceLastQuery']['packetsSentUpload'])

		# Process Network-Level Tin Statistics
		tinsStats = buildTinStats(tinsStats, allPacketsDownload, allPacketsUpload)
		
		return subscriberCircuits, tinsStats
	
	except exceptionWithMessage:
		print("There was an exception but it was caught and sent to Spike. Will try again next time around.")
	except Exception as e:
		exceptionWithMessage("getCircuitBandwidthStats: {}".format(e))
	except:
		exceptionWithMessage("getCircuitBandwidthStats")

def buildTinStats(data, allPacketsDownload, allPacketsUpload):
	# data is the dictionary that we want to do the calculations for, usually circuit['tinsStats'] or tinsStats.
	allPackets = {
		"Download": allPacketsDownload,
		"Upload": allPacketsUpload
	}
	for tinK, tinV in data['sinceLastQuery'].items():
		for directionK, directionV in tinV.items():
			try:
				currentQuerySentPackets = data['currentQuery'][tinK][directionK]['sent_packets'] if data['currentQuery'][tinK][directionK]['sent_packets'] > 0 else 0.0
			except KeyError:
				currentQuerySentPackets = 0.0
			try:
				priorQuerySentPackets = data['priorQuery'][tinK][directionK]['sent_packets'] if data['priorQuery'][tinK][directionK]['sent_packets'] > 0 else 0.0
			except KeyError:
				priorQuerySentPackets = 0.0
			try:
				data['sinceLastQuery'][tinK][directionK]['sent_packets'] = currentQuerySentPackets - priorQuerySentPackets
			except Exception as e:
				exceptionWithMessage("QoE Tins Sent Packet Broken: {}".format(e))

			try:
				currentQueryDrops = data['currentQuery'][tinK][directionK]['drops'] if data['currentQuery'][tinK][directionK]['drops'] > 0 else 0.0
			except KeyError:
				currentQueryDrops = 0.0
			try:
				priorQueryDrops = data['priorQuery'][tinK][directionK]['drops'] if data['priorQuery'][tinK][directionK]['drops'] > 0 else 0.0
			except KeyError:
				priorQueryDrops = 0.0
			try:	
				data['sinceLastQuery'][tinK][directionK]['drops'] = currentQueryDrops - priorQueryDrops
			except Exception as e:
				exceptionWithMessage("QoE Tins Drops Broken: {}".format(e))

			try:
				directionPercentage = directionV['drops'] / directionV['sent_packets'] if directionV['sent_packets'] > 0 else 0.0
			except KeyError:
				directionPercentage = 0.0
			try:
				data['sinceLastQuery'][tinK][directionK]['dropPercentage'] = max(round(directionPercentage * 100.0, 3), 0.0)
			except Exception as e:
				exceptionWithMessage("QoE Tins Drop Percentage Broken: {}".format(e))
			
			try:
				data['sinceLastQuery'][tinK][directionK]['utilizationPercentage'] = min(round((directionV['sent_packets'] / allPackets[directionK]) * 100.0, 3), 100.0) if allPackets[directionK] > 0 else 0.0
			except Exception as e:
				exceptionWithMessage("QoE Tins Percentage Utilization Broken: {}".format(e))

	return data

def getParentNodeBandwidthStats(parentNodes, subscriberCircuits):
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
		thisParentNodeStats = {'sinceLastQuery': {}}
		for circuit in subscriberCircuits:
			if circuit['ParentNode'] == parentNode['parentNodeName']:
				thisNodeBitsDownload += circuit['stats']['sinceLastQuery']['bitsDownload']
				thisNodeBitsUpload += circuit['stats']['sinceLastQuery']['bitsUpload']
				#thisNodeDropsDownload += circuit['packetDropsDownloadSinceLastQuery']
				#thisNodeDropsUpload += circuit['packetDropsUploadSinceLastQuery']
				thisNodeDropsTotal += (circuit['stats']['sinceLastQuery']['packetDropsDownload'] + circuit['stats']['sinceLastQuery']['packetDropsUpload'])
				packetsSentDownloadAggregate += circuit['stats']['sinceLastQuery']['packetsSentDownload']
				packetsSentUploadAggregate += circuit['stats']['sinceLastQuery']['packetsSentUpload']
				packetsSentTotalAggregate += (circuit['stats']['sinceLastQuery']['packetsSentDownload'] + circuit['stats']['sinceLastQuery']['packetsSentUpload'])
				circuitsMatched += 1
		if (packetsSentDownloadAggregate > 0) and (packetsSentUploadAggregate > 0):
			#overloadFactorDownloadSinceLastQuery = float(round((thisNodeDropsDownload/packetsSentDownloadAggregate)*100.0, 3))
			#overloadFactorUploadSinceLastQuery = float(round((thisNodeDropsUpload/packetsSentUploadAggregate)*100.0, 3))
			overloadFactorTotalSinceLastQuery = float(round((thisNodeDropsTotal/packetsSentTotalAggregate)*100.0, 1))
		else:
			#overloadFactorDownloadSinceLastQuery = 0.0
			#overloadFactorUploadSinceLastQuery = 0.0
			overloadFactorTotalSinceLastQuery = 0.0
		
		thisParentNodeStats['sinceLastQuery']['bitsDownload'] = thisNodeBitsDownload
		thisParentNodeStats['sinceLastQuery']['bitsUpload'] = thisNodeBitsUpload
		thisParentNodeStats['sinceLastQuery']['packetDropsTotal'] = thisNodeDropsTotal
		thisParentNodeStats['sinceLastQuery']['overloadFactorTotal'] = overloadFactorTotalSinceLastQuery
		parentNode['stats'] = thisParentNodeStats
		
	return parentNodes


def getParentNodeLatencyStats(parentNodes, subscriberCircuits):
	for parentNode in parentNodes:
		if 'stats' not in parentNode:
			parentNode['stats'] = {}
			parentNode['stats']['sinceLastQuery'] = {}
	
	for parentNode in parentNodes:
		thisParentNodeStats = {'sinceLastQuery': {}}
		circuitsMatchedLatencies = []
		for circuit in subscriberCircuits:
			if circuit['ParentNode'] == parentNode['parentNodeName']:
				if circuit['stats']['sinceLastQuery']['tcpLatency'] != None:
					circuitsMatchedLatencies.append(circuit['stats']['sinceLastQuery']['tcpLatency'])
		if len(circuitsMatchedLatencies) > 0:
			thisParentNodeStats['sinceLastQuery']['tcpLatency'] = statistics.median(circuitsMatchedLatencies)
		else:
			thisParentNodeStats['sinceLastQuery']['tcpLatency'] = None
		parentNode['stats'] = thisParentNodeStats
	return parentNodes


def getCircuitLatencyStats(subscriberCircuits):
	command = './bin/xdp_pping'
	consoleOutput = subprocess.run(command.split(' '), stdout=subprocess.PIPE).stdout.decode('utf-8')
	consoleOutput = consoleOutput.replace('\n','').replace('}{', '}, {')
	listOfEntries = json.loads(consoleOutput)
	
	tcpLatencyForClassID = {}
	for entry in listOfEntries:
		if 'tc' in entry:
			handle = '0x' + entry['tc'].split(':')[0] + ':' + '0x' + entry['tc'].split(':')[1]
			# To avoid outliers messing up avg for each circuit - cap at ceiling of 200ms
			ceiling = 200.0
			tcpLatencyForClassID[handle] = min(entry['median'], ceiling)
	for circuit in subscriberCircuits:
		if 'stats' not in circuit:
			circuit['stats'] = {}
			circuit['stats']['sinceLastQuery'] = {}
			
	for circuit in subscriberCircuits:
		classID = circuit['classid']
		if classID in tcpLatencyForClassID:
			circuit['stats']['sinceLastQuery']['tcpLatency'] = tcpLatencyForClassID[classID]
		else:
			circuit['stats']['sinceLastQuery']['tcpLatency'] = None
			try:
				if circuit['stats']['priorQuery'] != None:
					if 'priorQuery' in circuit['stats']:
						if 'tcpLatency' in circuit['stats']['priorQuery']:
							circuit['stats']['sinceLastQuery']['tcpLatency'] = circuit['stats']['priorQuery']['tcpLatency']
			except:
				circuit['stats']['sinceLastQuery']['tcpLatency'] = None
				# priorQuery had no latency information, using None instead
	return subscriberCircuits


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
	# Load network hierarchy
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
							
	fileLoc = Path("tinsStats.json")
	if fileLoc.is_file():
		with open(fileLoc, 'r') as j:
			tinsStats = json.loads(j.read())
	else:
		tinsStats =	{}					
							
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
	subscriberCircuits, tinsStats = getCircuitBandwidthStats(subscriberCircuits, tinsStats)
	print("Computing parent node statistics")
	parentNodes = getParentNodeBandwidthStats(parentNodes, subscriberCircuits)
	print("Writing data to InfluxDB")
	client = InfluxDBClient(
		url=influxDBurl,
		token=influxDBtoken,
		org=influxDBOrg
	)
	
	# Record current timestamp, use for all points added
	timestamp = time.time_ns()
	write_api = client.write_api(write_options=SYNCHRONOUS)

	chunkedsubscriberCircuits = list(chunk_list(subscriberCircuits, 200))

	queriesToSendCount = 0
	for chunk in chunkedsubscriberCircuits:
		queriesToSend = []
		for circuit in chunk:
			bitsDownloadMin = float(circuit['minDownload']) * 1000000
			bitsDownloadMax = float(circuit['maxDownload']) * 1000000
			bitsUploadMin = float(circuit['minUpload']) * 1000000
			bitsUploadMax = float(circuit['maxUpload']) * 1000000
			bitsDownload = float(circuit['stats']['sinceLastQuery']['bitsDownload'])
			bitsUpload = float(circuit['stats']['sinceLastQuery']['bitsUpload'])
			bytesSentDownload = float(circuit['stats']['sinceLastQuery']['bytesSentDownload'])
			bytesSentUpload = float(circuit['stats']['sinceLastQuery']['bytesSentUpload'])
			percentUtilizationDownload = round((bitsDownload / round(circuit['maxDownload'] * 1000000))*100.0, 1)
			percentUtilizationUpload = round((bitsUpload / round(circuit['maxUpload'] * 1000000))*100.0, 1)
			p = Point('Bandwidth').tag("CircuitID", circuit['circuitID']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", bitsDownload).field("Upload", bitsUpload).field("Download Minimum", bitsDownloadMin).field("Download Maximum", bitsDownloadMax).field("Upload Minimum", bitsUploadMin).field("Upload Maximum", bitsUploadMax).time(timestamp)
			queriesToSend.append(p)
			p = Point('Utilization').tag("CircuitID", circuit['circuitID']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload).time(timestamp)
			queriesToSend.append(p)
			p = Point('BandwidthUsage').tag("CircuitID", circuit['circuitID']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", bytesSentDownload).field("Upload", bytesSentUpload).time(timestamp)
			queriesToSend.append(p)
			# Parse tins by circuit and parent node and ship to InfluxDB
			for tinK, tinV in circuit['tinsStats']['sinceLastQuery'].items():
				for directionK, directionV in tinV.items():
					for metricK, metricV in directionV.items():
						p = Point('Tins By Circuit and Parent Node').tag("CircuitID", circuit['circuitID']).tag("ParentNode", circuit['ParentNode']).tag("Tin", tinK).tag("Direction", directionK).field(metricK, metricV).time(timestamp)
						queriesToSend.append(p)

		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	for parentNode in parentNodes:
		bitsDownload = float(parentNode['stats']['sinceLastQuery']['bitsDownload'])
		bitsUpload = float(parentNode['stats']['sinceLastQuery']['bitsUpload'])
		dropsTotal = float(parentNode['stats']['sinceLastQuery']['packetDropsTotal'])
		overloadFactor = float(parentNode['stats']['sinceLastQuery']['overloadFactorTotal'])
		droppedPacketsAllTime += dropsTotal
		percentUtilizationDownload = round((bitsDownload / round(parentNode['maxDownload'] * 1000000))*100.0, 1)
		percentUtilizationUpload = round((bitsUpload / round(parentNode['maxUpload'] * 1000000))*100.0, 1)
		p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload).field("Upload", bitsUpload).time(timestamp)
		queriesToSend.append(p)
		p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload).time(timestamp)
		queriesToSend.append(p)
		p = Point('Overload').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Overload", overloadFactor).time(timestamp)
		queriesToSend.append(p)

	write_api.write(bucket=influxDBBucket, record=queriesToSend)
	# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	queriesToSendCount += len(queriesToSend)
	
	if 'cake diffserv4' in sqm:
		queriesToSend = []
		listOfTins = ['Bulk', 'BestEffort', 'Video', 'Voice']
		for tin in listOfTins:
			p = Point('Tin Drop Percentage').tag("Type", "Tin").tag("Tin", tin).field("Download", tinsStats['sinceLastQuery'][tin]['Download']['dropPercentage']).field("Upload", tinsStats['sinceLastQuery'][tin]['Upload']['dropPercentage']).time(timestamp)
			queriesToSend.append(p)
			p = Point('Tins Utilization').tag("Type", "Tin").tag("Tin", tin).field("Download", tinsStats['sinceLastQuery'][tin]['Download']['utilizationPercentage']).field("Upload", tinsStats['sinceLastQuery'][tin]['Upload']['utilizationPercentage']).time(timestamp)
			queriesToSend.append(p)

		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)
	
	# Graph CPU use
	cpuVals = psutil.cpu_percent(percpu=True)
	queriesToSend = []
	for index, item in enumerate(cpuVals):
		p = Point('CPU').field('CPU_' + str(index), item)
		queriesToSend.append(p)
	write_api.write(bucket=influxDBBucket, record=queriesToSend)
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
		
	with open('tinsStats.json', 'w') as f:
		f.write(json.dumps(tinsStats, indent=4))

	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(), 2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")

def refreshLatencyGraphs():
	startTime = datetime.now()
	with open('statsByParentNode.json', 'r') as j:
		parentNodes = json.loads(j.read())

	with open('statsByCircuit.json', 'r') as j:
		subscriberCircuits = json.loads(j.read())

	parentNodeNameDict = parentNodeNameDictPull()

	print("Retrieving circuit statistics")
	subscriberCircuits = getCircuitLatencyStats(subscriberCircuits)
	print("Computing parent node statistics")
	parentNodes = getParentNodeLatencyStats(parentNodes, subscriberCircuits)
	print("Writing data to InfluxDB")
	client = InfluxDBClient(
		url=influxDBurl,
		token=influxDBtoken,
		org=influxDBOrg
	)
	
	# Record current timestamp, use for all points added
	timestamp = time.time_ns()
	
	write_api = client.write_api(write_options=SYNCHRONOUS)

	chunkedsubscriberCircuits = list(chunk_list(subscriberCircuits, 200))

	queriesToSendCount = 0
	for chunk in chunkedsubscriberCircuits:
		queriesToSend = []
		for circuit in chunk:
			if circuit['stats']['sinceLastQuery']['tcpLatency'] != None:
				tcpLatency = float(circuit['stats']['sinceLastQuery']['tcpLatency'])
				p = Point('TCP Latency').tag("CircuitID", circuit['circuitID']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("TCP Latency", tcpLatency).time(timestamp)
				queriesToSend.append(p)
		write_api.write(bucket=influxDBBucket, record=queriesToSend)
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	for parentNode in parentNodes:
		if parentNode['stats']['sinceLastQuery']['tcpLatency'] != None:
			tcpLatency = float(parentNode['stats']['sinceLastQuery']['tcpLatency'])
			p = Point('TCP Latency').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("TCP Latency", tcpLatency).time(timestamp)
			queriesToSend.append(p)
	
	write_api.write(bucket=influxDBBucket, record=queriesToSend)
	queriesToSendCount += len(queriesToSend)
	
	listOfAllLatencies = []
	for circuit in subscriberCircuits:
		if circuit['stats']['sinceLastQuery']['tcpLatency'] != None:
			listOfAllLatencies.append(circuit['stats']['sinceLastQuery']['tcpLatency'])
	if len(listOfAllLatencies) > 0:
		currentNetworkLatency = float(statistics.median(listOfAllLatencies))
		p = Point('TCP Latency').tag("Type", "Network").field("TCP Latency", currentNetworkLatency).time(timestamp)
		write_api.write(bucket=influxDBBucket, record=p)
		queriesToSendCount += 1
	
	print("Added " + str(queriesToSendCount) + " points to InfluxDB.")

	client.close()
	
	with open('statsByParentNode.json', 'w') as f:
		f.write(json.dumps(parentNodes, indent=4))

	with open('statsByCircuit.json', 'w') as f:
		f.write(json.dumps(subscriberCircuits, indent=4))

	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds(), 2)
	print("Graphs updated within " + str(durationSeconds) + " seconds.")

if __name__ == '__main__':
	refreshBandwidthGraphs()
	refreshLatencyGraphs()
