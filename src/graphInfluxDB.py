import subprocess
import json
import subprocess
from datetime import datetime
from pathlib import Path
import statistics
import time
import psutil

from influxdb_client import InfluxDBClient, Point
from influxdb_client.client.write_api import SYNCHRONOUS

from liblqos_python import interface_a, interface_b, influx_db_enabled, influx_db_bucket, influx_db_org, influx_db_token, influx_db_url, sqm

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
	interfaces = [interface_a(), interface_b()]
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
		for (interface, stats, dirSuffix) in zip(interfaces, ifaceStats, ['Download', 'Upload']):

			element = stats[circuit['classid']] if circuit['classid'] in stats else False

			if element:
				bytesSent = float(element['bytes'])
				drops = float(element['drops'])
				packets = float(element['packets'])
				if (element['drops'] > 0) and (element['packets'] > 0):
					overloadFactor = float(round(element['drops']/element['packets'],3))
				else:
					overloadFactor = 0.0
				
				if 'cake diffserv4' in sqm():
					tinCounter = 1
					for tin in element['tins']:
						sent_packets = float(tin['sent_packets'])
						ack_drops = float(tin['ack_drops'])
						ecn_mark = float(tin['ecn_mark'])
						tinDrops = float(tin['drops'])
						trueDrops = ecn_mark + tinDrops - ack_drops
						if tinCounter == 1:
							tinsStats['currentQuery']['Bulk'][dirSuffix]['sent_packets'] += sent_packets
							tinsStats['currentQuery']['Bulk'][dirSuffix]['drops'] += trueDrops
						elif tinCounter == 2:
							tinsStats['currentQuery']['BestEffort'][dirSuffix]['sent_packets'] += sent_packets
							tinsStats['currentQuery']['BestEffort'][dirSuffix]['drops'] += trueDrops
						elif tinCounter == 3:
							tinsStats['currentQuery']['Video'][dirSuffix]['sent_packets'] += sent_packets
							tinsStats['currentQuery']['Video'][dirSuffix]['drops'] += trueDrops
						elif tinCounter == 4:
							tinsStats['currentQuery']['Voice'][dirSuffix]['sent_packets'] += sent_packets
							tinsStats['currentQuery']['Voice'][dirSuffix]['drops'] += trueDrops
						tinCounter += 1

				circuit['stats']['currentQuery']['bytesSent' + dirSuffix] = bytesSent
				circuit['stats']['currentQuery']['packetDrops' + dirSuffix] = drops
				circuit['stats']['currentQuery']['packetsSent' + dirSuffix] = packets
				circuit['stats']['currentQuery']['overloadFactor' + dirSuffix] = overloadFactor
				
				#if 'cake diffserv4' in sqm():
				#	circuit['stats']['currentQuery']['tins'] = theseTins

		circuit['stats']['currentQuery']['time'] = datetime.now().isoformat()
		
	allPacketsDownload = 0.0
	allPacketsUpload = 0.0
	for circuit in subscriberCircuits:
		circuit['stats']['sinceLastQuery']['bitsDownload'] = circuit['stats']['sinceLastQuery']['bitsUpload'] = None
		circuit['stats']['sinceLastQuery']['bytesSentDownload'] = circuit['stats']['sinceLastQuery']['bytesSentUpload'] = None
		circuit['stats']['sinceLastQuery']['packetDropsDownload'] = circuit['stats']['sinceLastQuery']['packetDropsUpload'] = None
		circuit['stats']['sinceLastQuery']['packetsSentDownload'] = circuit['stats']['sinceLastQuery']['packetsSentUpload'] = None
		
		try:
			if (circuit['stats']['currentQuery']['bytesSentDownload'] - circuit['stats']['priorQuery']['bytesSentDownload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['bytesSentDownload'] = circuit['stats']['currentQuery']['bytesSentDownload'] - circuit['stats']['priorQuery']['bytesSentDownload']
			else:
				circuit['stats']['sinceLastQuery']['bytesSentDownload'] = None
			if (circuit['stats']['currentQuery']['bytesSentUpload'] - circuit['stats']['priorQuery']['bytesSentUpload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['bytesSentUpload'] = circuit['stats']['currentQuery']['bytesSentUpload'] - circuit['stats']['priorQuery']['bytesSentUpload']
			else:
				circuit['stats']['sinceLastQuery']['bytesSentUpload'] = None
		except:
			circuit['stats']['sinceLastQuery']['bytesSentDownload'] = None
			circuit['stats']['sinceLastQuery']['bytesSentUpload'] = None
		try:
			if (circuit['stats']['currentQuery']['packetDropsDownload'] - circuit['stats']['priorQuery']['packetDropsDownload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['packetDropsDownload'] = circuit['stats']['currentQuery']['packetDropsDownload'] - circuit['stats']['priorQuery']['packetDropsDownload']
			else:
				circuit['stats']['sinceLastQuery']['packetDropsDownload'] = None
			if (circuit['stats']['currentQuery']['packetDropsUpload'] - circuit['stats']['priorQuery']['packetDropsUpload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['packetDropsUpload'] = circuit['stats']['currentQuery']['packetDropsUpload'] - circuit['stats']['priorQuery']['packetDropsUpload']
			else:
				circuit['stats']['sinceLastQuery']['packetDropsUpload'] = None
		except:
			circuit['stats']['sinceLastQuery']['packetDropsDownload'] = None
			circuit['stats']['sinceLastQuery']['packetDropsUpload'] = None
		try:
			if (circuit['stats']['currentQuery']['packetsSentDownload'] - circuit['stats']['priorQuery']['packetsSentDownload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['packetsSentDownload'] = circuit['stats']['currentQuery']['packetsSentDownload'] - circuit['stats']['priorQuery']['packetsSentDownload']
			else:
				circuit['stats']['sinceLastQuery']['packetsSentDownload'] = None
			if (circuit['stats']['currentQuery']['packetsSentUpload'] - circuit['stats']['priorQuery']['packetsSentUpload']) >= 0.0:
				circuit['stats']['sinceLastQuery']['packetsSentUpload'] = circuit['stats']['currentQuery']['packetsSentUpload'] - circuit['stats']['priorQuery']['packetsSentUpload']
			else:
				circuit['stats']['sinceLastQuery']['packetsSentUpload'] = None
		except:
			circuit['stats']['sinceLastQuery']['packetsSentDownload'] = None
			circuit['stats']['sinceLastQuery']['packetsSentUpload'] = None
		
		if(circuit['stats']['sinceLastQuery']['packetsSentDownload']):
			allPacketsDownload += circuit['stats']['sinceLastQuery']['packetsSentDownload']
		if(circuit['stats']['sinceLastQuery']['packetsSentUpload']):
			allPacketsUpload += circuit['stats']['sinceLastQuery']['packetsSentUpload']
		
		if 'priorQuery' in circuit['stats']:
			if 'time' in circuit['stats']['priorQuery']:
				currentQueryTime = datetime.fromisoformat(circuit['stats']['currentQuery']['time'])
				priorQueryTime = datetime.fromisoformat(circuit['stats']['priorQuery']['time'])
				deltaSeconds = (currentQueryTime - priorQueryTime).total_seconds()
				if (circuit['stats']['sinceLastQuery']['bytesSentDownload']):
					circuit['stats']['sinceLastQuery']['bitsDownload'] = round((circuit['stats']['sinceLastQuery']['bytesSentDownload'] * 8) / deltaSeconds) if deltaSeconds > 0 else 0
				else:
					circuit['stats']['sinceLastQuery']['bitsDownload'] = None
				if (circuit['stats']['sinceLastQuery']['bytesSentUpload']):
					circuit['stats']['sinceLastQuery']['bitsUpload'] = round((circuit['stats']['sinceLastQuery']['bytesSentUpload'] * 8) / deltaSeconds) if deltaSeconds > 0 else 0
				else:
					circuit['stats']['sinceLastQuery']['bitsUpload'] = None
				
		else:
			circuit['stats']['sinceLastQuery']['bitsDownload'] = None
			if(circuit['stats']['sinceLastQuery']['bytesSentDownload']):
				circuit['stats']['sinceLastQuery']['bitsDownload'] = (circuit['stats']['sinceLastQuery']['bytesSentDownload'] * 8)
			circuit['stats']['sinceLastQuery']['bitsUpload'] = None
			if(circuit['stats']['sinceLastQuery']['bytesSentUpload']):
				circuit['stats']['sinceLastQuery']['bitsUpload'] = (circuit['stats']['sinceLastQuery']['bytesSentUpload'] * 8)
	
	tinsStats['sinceLastQuery']['Bulk']['Download']['dropPercentage'] = tinsStats['sinceLastQuery']['Bulk']['Upload']['dropPercentage'] = 0.0
	tinsStats['sinceLastQuery']['BestEffort']['Download']['dropPercentage'] = tinsStats['sinceLastQuery']['BestEffort']['Upload']['dropPercentage'] = 0.0
	tinsStats['sinceLastQuery']['Video']['Download']['dropPercentage'] = tinsStats['sinceLastQuery']['Video']['Upload']['dropPercentage'] = 0.0
	tinsStats['sinceLastQuery']['Voice']['Download']['dropPercentage'] = tinsStats['sinceLastQuery']['Voice']['Upload']['dropPercentage'] = 0.0
	
	tinsStats['sinceLastQuery']['Bulk']['Download']['percentage'] = tinsStats['sinceLastQuery']['Bulk']['Upload']['percentage'] = 0.0
	tinsStats['sinceLastQuery']['BestEffort']['Download']['percentage'] = tinsStats['sinceLastQuery']['BestEffort']['Upload']['percentage'] = 0.0
	tinsStats['sinceLastQuery']['Video']['Download']['percentage'] = tinsStats['sinceLastQuery']['Video']['Upload']['percentage'] = 0.0
	tinsStats['sinceLastQuery']['Voice']['Download']['percentage'] = tinsStats['sinceLastQuery']['Voice']['Upload']['percentage'] = 0.0
	
	try:
		tinsStats['sinceLastQuery']['Bulk']['Download']['sent_packets'] = tinsStats['currentQuery']['Bulk']['Download']['sent_packets'] - tinsStats['priorQuery']['Bulk']['Download']['sent_packets']
		tinsStats['sinceLastQuery']['BestEffort']['Download']['sent_packets'] = tinsStats['currentQuery']['BestEffort']['Download']['sent_packets'] - tinsStats['priorQuery']['BestEffort']['Download']['sent_packets']
		tinsStats['sinceLastQuery']['Video']['Download']['sent_packets'] = tinsStats['currentQuery']['Video']['Download']['sent_packets'] - tinsStats['priorQuery']['Video']['Download']['sent_packets']
		tinsStats['sinceLastQuery']['Voice']['Download']['sent_packets'] = tinsStats['currentQuery']['Voice']['Download']['sent_packets'] - tinsStats['priorQuery']['Voice']['Download']['sent_packets']
		tinsStats['sinceLastQuery']['Bulk']['Upload']['sent_packets'] = tinsStats['currentQuery']['Bulk']['Upload']['sent_packets'] - tinsStats['priorQuery']['Bulk']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['BestEffort']['Upload']['sent_packets'] = tinsStats['currentQuery']['BestEffort']['Upload']['sent_packets'] - tinsStats['priorQuery']['BestEffort']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['Video']['Upload']['sent_packets'] = tinsStats['currentQuery']['Video']['Upload']['sent_packets'] - tinsStats['priorQuery']['Video']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['Voice']['Upload']['sent_packets'] = tinsStats['currentQuery']['Voice']['Upload']['sent_packets'] - tinsStats['priorQuery']['Voice']['Upload']['sent_packets']
	except:
		tinsStats['sinceLastQuery']['Bulk']['Download']['sent_packets'] = tinsStats['sinceLastQuery']['BestEffort']['Download']['sent_packets'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Download']['sent_packets'] = tinsStats['sinceLastQuery']['Voice']['Download']['sent_packets'] = 0.0
		tinsStats['sinceLastQuery']['Bulk']['Upload']['sent_packets'] = tinsStats['sinceLastQuery']['BestEffort']['Upload']['sent_packets'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Upload']['sent_packets'] = tinsStats['sinceLastQuery']['Voice']['Upload']['sent_packets'] = 0.0

	try:
		tinsStats['sinceLastQuery']['Bulk']['Download']['drops'] = tinsStats['currentQuery']['Bulk']['Download']['drops'] - tinsStats['priorQuery']['Bulk']['Download']['drops']
		tinsStats['sinceLastQuery']['BestEffort']['Download']['drops'] = tinsStats['currentQuery']['BestEffort']['Download']['drops'] - tinsStats['priorQuery']['BestEffort']['Download']['drops']
		tinsStats['sinceLastQuery']['Video']['Download']['drops'] = tinsStats['currentQuery']['Video']['Download']['drops'] - tinsStats['priorQuery']['Video']['Download']['drops']
		tinsStats['sinceLastQuery']['Voice']['Download']['drops'] = tinsStats['currentQuery']['Voice']['Download']['drops'] - tinsStats['priorQuery']['Voice']['Download']['drops']
		tinsStats['sinceLastQuery']['Bulk']['Upload']['drops'] = tinsStats['currentQuery']['Bulk']['Upload']['drops'] - tinsStats['priorQuery']['Bulk']['Upload']['drops']
		tinsStats['sinceLastQuery']['BestEffort']['Upload']['drops'] = tinsStats['currentQuery']['BestEffort']['Upload']['drops'] - tinsStats['priorQuery']['BestEffort']['Upload']['drops']
		tinsStats['sinceLastQuery']['Video']['Upload']['drops'] = tinsStats['currentQuery']['Video']['Upload']['drops'] - tinsStats['priorQuery']['Video']['Upload']['drops']
		tinsStats['sinceLastQuery']['Voice']['Upload']['drops'] = tinsStats['currentQuery']['Voice']['Upload']['drops'] - tinsStats['priorQuery']['Voice']['Upload']['drops']
	except:
		tinsStats['sinceLastQuery']['Bulk']['Download']['drops'] = tinsStats['sinceLastQuery']['BestEffort']['Download']['drops'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Download']['drops'] = tinsStats['sinceLastQuery']['Voice']['Download']['drops'] = 0.0
		tinsStats['sinceLastQuery']['Bulk']['Upload']['drops'] = tinsStats['sinceLastQuery']['BestEffort']['Upload']['drops'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Upload']['drops'] = tinsStats['sinceLastQuery']['Voice']['Upload']['drops'] = 0.0

	try:
		dlPerc = tinsStats['sinceLastQuery']['Bulk']['Download']['drops'] / tinsStats['sinceLastQuery']['Bulk']['Download']['sent_packets']
		ulPerc = tinsStats['sinceLastQuery']['Bulk']['Upload']['drops'] / tinsStats['sinceLastQuery']['Bulk']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['Bulk']['Download']['dropPercentage'] = max(round(dlPerc * 100.0, 3),0.0)
		tinsStats['sinceLastQuery']['Bulk']['Upload']['dropPercentage'] = max(round(ulPerc * 100.0, 3),0.0)
		
		dlPerc = tinsStats['sinceLastQuery']['BestEffort']['Download']['drops'] / tinsStats['sinceLastQuery']['BestEffort']['Download']['sent_packets']
		ulPerc = tinsStats['sinceLastQuery']['BestEffort']['Upload']['drops'] / tinsStats['sinceLastQuery']['BestEffort']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['BestEffort']['Download']['dropPercentage'] = max(round(dlPerc * 100.0, 3),0.0)
		tinsStats['sinceLastQuery']['BestEffort']['Upload']['dropPercentage'] = max(round(ulPerc * 100.0, 3),0.0)
		
		dlPerc = tinsStats['sinceLastQuery']['Video']['Download']['drops'] / tinsStats['sinceLastQuery']['Video']['Download']['sent_packets']
		ulPerc = tinsStats['sinceLastQuery']['Video']['Upload']['drops'] / tinsStats['sinceLastQuery']['Video']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['Video']['Download']['dropPercentage'] = max(round(dlPerc * 100.0, 3),0.0)
		tinsStats['sinceLastQuery']['Video']['Upload']['dropPercentage'] = max(round(ulPerc * 100.0, 3),0.0)
		
		dlPerc = tinsStats['sinceLastQuery']['Voice']['Download']['drops'] / tinsStats['sinceLastQuery']['Voice']['Download']['sent_packets']
		ulPerc = tinsStats['sinceLastQuery']['Voice']['Upload']['drops'] / tinsStats['sinceLastQuery']['Voice']['Upload']['sent_packets']
		tinsStats['sinceLastQuery']['Voice']['Download']['dropPercentage'] = max(round(dlPerc * 100.0, 3),0.0)
		tinsStats['sinceLastQuery']['Voice']['Upload']['dropPercentage'] = max(round(ulPerc * 100.0, 3),0.0)
	except:
		tinsStats['sinceLastQuery']['Bulk']['Download']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['Bulk']['Upload']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['BestEffort']['Download']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['BestEffort']['Upload']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Download']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['Video']['Upload']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['Voice']['Download']['dropPercentage'] = 0.0
		tinsStats['sinceLastQuery']['Voice']['Upload']['dropPercentage'] = 0.0
		
	try:
		tinsStats['sinceLastQuery']['Bulk']['Download']['percentage'] = min(round((tinsStats['sinceLastQuery']['Bulk']['Download']['sent_packets']/allPacketsUpload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['Bulk']['Upload']['percentage'] = min(round((tinsStats['sinceLastQuery']['Bulk']['Upload']['sent_packets']/allPacketsUpload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['BestEffort']['Download']['percentage'] = min(round((tinsStats['sinceLastQuery']['BestEffort']['Download']['sent_packets']/allPacketsDownload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['BestEffort']['Upload']['percentage'] = min(round((tinsStats['sinceLastQuery']['BestEffort']['Upload']['sent_packets']/allPacketsUpload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['Video']['Download']['percentage'] = min(round((tinsStats['sinceLastQuery']['Video']['Download']['sent_packets']/allPacketsDownload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['Video']['Upload']['percentage'] = min(round((tinsStats['sinceLastQuery']['Video']['Upload']['sent_packets']/allPacketsUpload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['Voice']['Download']['percentage'] = min(round((tinsStats['sinceLastQuery']['Voice']['Download']['sent_packets']/allPacketsDownload)*100.0, 3),100.0)
		tinsStats['sinceLastQuery']['Voice']['Upload']['percentage'] = min(round((tinsStats['sinceLastQuery']['Voice']['Upload']['sent_packets']/allPacketsUpload)*100.0, 3),100.0)
	except:
		# To avoid graphing 0.0 for all categories, which would show unusual graph results upon each queue reload, we just set these to None if the above calculations fail.
		tinsStats['sinceLastQuery']['Bulk']['Download']['percentage'] = tinsStats['sinceLastQuery']['Bulk']['Upload']['percentage'] = None
		tinsStats['sinceLastQuery']['BestEffort']['Download']['percentage'] = tinsStats['sinceLastQuery']['BestEffort']['Upload']['percentage'] = None
		tinsStats['sinceLastQuery']['Video']['Download']['percentage'] = tinsStats['sinceLastQuery']['Video']['Upload']['percentage'] = None
		tinsStats['sinceLastQuery']['Voice']['Download']['percentage'] = tinsStats['sinceLastQuery']['Voice']['Upload']['percentage'] = None
	
	return subscriberCircuits, tinsStats


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
				if circuit['stats']['sinceLastQuery']['bitsDownload']:
					thisNodeBitsDownload += circuit['stats']['sinceLastQuery']['bitsDownload']
				if circuit['stats']['sinceLastQuery']['bitsUpload']:
					thisNodeBitsUpload += circuit['stats']['sinceLastQuery']['bitsUpload']
				#thisNodeDropsDownload += circuit['packetDropsDownloadSinceLastQuery']
				#thisNodeDropsUpload += circuit['packetDropsUploadSinceLastQuery']
				if circuit['stats']['sinceLastQuery']['packetDropsDownload'] and circuit['stats']['sinceLastQuery']['packetDropsUpload']:
					thisNodeDropsTotal += (circuit['stats']['sinceLastQuery']['packetDropsDownload'] + circuit['stats']['sinceLastQuery']['packetDropsUpload'])
				if circuit['stats']['sinceLastQuery']['packetsSentDownload']:
					packetsSentDownloadAggregate += circuit['stats']['sinceLastQuery']['packetsSentDownload']
				if circuit['stats']['sinceLastQuery']['packetsSentUpload']:
					packetsSentUploadAggregate += circuit['stats']['sinceLastQuery']['packetsSentUpload']
				if circuit['stats']['sinceLastQuery']['packetsSentDownload'] and circuit['stats']['sinceLastQuery']['packetsSentUpload']:
					packetsSentTotalAggregate += (circuit['stats']['sinceLastQuery']['packetsSentDownload'] + circuit['stats']['sinceLastQuery']['packetsSentUpload'])
				circuitsMatched += 1
		if (packetsSentDownloadAggregate > 0) and (packetsSentUploadAggregate > 0) and (packetsSentTotalAggregate > 0):
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
			# If we can't identify RTT this time around, use most recently recorded RTT
			# None by default, change if found in priorQuery
			circuit['stats']['sinceLastQuery']['tcpLatency'] = None
			if 'priorQuery' in circuit['stats']:
				if circuit['stats']['priorQuery'] != None:
					if 'priorQuery' in circuit['stats']:
						if 'tcpLatency' in circuit['stats']['priorQuery']:
							circuit['stats']['sinceLastQuery']['tcpLatency'] = circuit['stats']['priorQuery']['tcpLatency']

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
		url=influx_db_url(),
		token=influx_db_token(),
		org=influx_db_org()
	)
	
	# Record current timestamp, use for all points added
	timestamp = time.time_ns()
	write_api = client.write_api(write_options=SYNCHRONOUS)

	chunkedsubscriberCircuits = list(chunk_list(subscriberCircuits, 200))

	queriesToSendCount = 0
	for chunk in chunkedsubscriberCircuits:
		seenSomethingBesides0s = False
		queriesToSend = []
		for circuit in chunk:
			bitsDownloadMin = float(circuit['minDownload']) * 1000000 if circuit['minDownload'] else None
			bitsDownloadMax = float(circuit['maxDownload']) * 1000000 if circuit['maxDownload'] else None
			bitsUploadMin = float(circuit['minUpload']) * 1000000 if circuit['minUpload'] else None
			bitsUploadMax = float(circuit['maxUpload']) * 1000000 if circuit['maxUpload'] else None
			bitsDownload = float(circuit['stats']['sinceLastQuery']['bitsDownload']) if circuit['stats']['sinceLastQuery']['bitsDownload'] else None
			bitsUpload = float(circuit['stats']['sinceLastQuery']['bitsUpload']) if circuit['stats']['sinceLastQuery']['bitsUpload'] else None
			bytesSentDownload = float(circuit['stats']['sinceLastQuery']['bytesSentDownload']) if circuit['stats']['sinceLastQuery']['bytesSentDownload'] else None
			bytesSentUpload = float(circuit['stats']['sinceLastQuery']['bytesSentUpload']) if circuit['stats']['sinceLastQuery']['bytesSentUpload'] else None
			percentUtilizationDownload = round((bitsDownload / round(circuit['maxDownload'] * 1000000))*100.0, 1) if bitsDownload and circuit['maxDownload'] else None
			percentUtilizationUpload = round((bitsUpload / round(circuit['maxUpload'] * 1000000))*100.0, 1) if bitsUpload and circuit['maxUpload'] else None
			if bitsDownload and bitsUpload:
				if (bitsDownload > 0.0) or (bitsUpload > 0.0):
					seenSomethingBesides0s = True
			p = Point('Bandwidth').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", bitsDownload).field("Upload", bitsUpload).time(timestamp)
			queriesToSend.append(p)
			p = Point('Utilization').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload).time(timestamp)
			queriesToSend.append(p)

		if seenSomethingBesides0s:
			write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	seenSomethingBesides0s = False
	for parentNode in parentNodes:
		bitsDownload = float(parentNode['stats']['sinceLastQuery']['bitsDownload'])
		bitsUpload = float(parentNode['stats']['sinceLastQuery']['bitsUpload'])
		dropsTotal = float(parentNode['stats']['sinceLastQuery']['packetDropsTotal'])
		overloadFactor = float(parentNode['stats']['sinceLastQuery']['overloadFactorTotal'])
		droppedPacketsAllTime += dropsTotal
		percentUtilizationDownload = round((bitsDownload / round(parentNode['maxDownload'] * 1000000))*100.0, 1)
		percentUtilizationUpload = round((bitsUpload / round(parentNode['maxUpload'] * 1000000))*100.0, 1)
		if bitsDownload and bitsUpload:
			if (bitsDownload > 0.0) or (bitsUpload > 0.0):
				seenSomethingBesides0s = True
		p = Point('Bandwidth').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", bitsDownload).field("Upload", bitsUpload).time(timestamp)
		queriesToSend.append(p)
		p = Point('Utilization').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Download", percentUtilizationDownload).field("Upload", percentUtilizationUpload).time(timestamp)
		queriesToSend.append(p)
		p = Point('Overload').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("Overload", overloadFactor).time(timestamp)
		queriesToSend.append(p)

	if seenSomethingBesides0s:
		write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
	# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	queriesToSendCount += len(queriesToSend)
	
	if 'cake diffserv4' in sqm():
		seenSomethingBesides0s = False
		queriesToSend = []
		listOfTins = ['Bulk', 'BestEffort', 'Video', 'Voice']
		for tin in listOfTins:
			p = Point('Tin Drop Percentage').tag("Type", "Tin").tag("Tin", tin).field("Download", tinsStats['sinceLastQuery'][tin]['Download']['dropPercentage']).field("Upload", tinsStats['sinceLastQuery'][tin]['Upload']['dropPercentage']).time(timestamp)
			queriesToSend.append(p)
			# Check to ensure tin percentage has value (!= None) before graphing. During partial or full reload these will have a value of None.
			if (tinsStats['sinceLastQuery'][tin]['Download']['percentage'] != None) and (tinsStats['sinceLastQuery'][tin]['Upload']['percentage'] != None):
				if (tinsStats['sinceLastQuery'][tin]['Download']['percentage'] > 0.0) or (tinsStats['sinceLastQuery'][tin]['Upload']['percentage'] > 0.0):
					seenSomethingBesides0s = True
				p = Point('Tins Assigned').tag("Type", "Tin").tag("Tin", tin).field("Download", tinsStats['sinceLastQuery'][tin]['Download']['percentage']).field("Upload", tinsStats['sinceLastQuery'][tin]['Upload']['percentage']).time(timestamp)
				queriesToSend.append(p)

		if seenSomethingBesides0s:
			write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
		# print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
		queriesToSendCount += len(queriesToSend)
	
	# Graph CPU use
	cpuVals = psutil.cpu_percent(percpu=True)
	queriesToSend = []
	for index, item in enumerate(cpuVals):
		p = Point('CPU').field('CPU_' + str(index), item)
		queriesToSend.append(p)
	write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
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
		url=influx_db_url(),
		token=influx_db_token(),
		org=influx_db_org()
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
				p = Point('TCP Latency').tag("Circuit", circuit['circuitName']).tag("ParentNode", circuit['ParentNode']).tag("Type", "Circuit").field("TCP Latency", tcpLatency).time(timestamp)
				queriesToSend.append(p)
		write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
		queriesToSendCount += len(queriesToSend)

	queriesToSend = []
	for parentNode in parentNodes:
		if parentNode['stats']['sinceLastQuery']['tcpLatency'] != None:
			tcpLatency = float(parentNode['stats']['sinceLastQuery']['tcpLatency'])
			p = Point('TCP Latency').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("TCP Latency", tcpLatency).time(timestamp)
			queriesToSend.append(p)
	
	write_api.write(bucket=influx_db_bucket(), record=queriesToSend)
	queriesToSendCount += len(queriesToSend)
	
	listOfAllLatencies = []
	for circuit in subscriberCircuits:
		if circuit['stats']['sinceLastQuery']['tcpLatency'] != None:
			listOfAllLatencies.append(circuit['stats']['sinceLastQuery']['tcpLatency'])
	if len(listOfAllLatencies) > 0:
		currentNetworkLatency = statistics.median(listOfAllLatencies)
		p = Point('TCP Latency').tag("Type", "Network").field("TCP Latency", currentNetworkLatency).time(timestamp)
		write_api.write(bucket=influx_db_bucket(), record=p)
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
