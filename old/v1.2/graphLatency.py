import os
import subprocess
from subprocess import PIPE
import io
import decimal
import json
from ispConfig import fqOrCAKE, interfaceA, interfaceB, influxDBBucket, influxDBOrg, influxDBtoken, influxDBurl, ppingLocation
from datetime import date, datetime, timedelta
import decimal
from influxdb_client import InfluxDBClient, Point, Dialect
from influxdb_client.client.write_api import SYNCHRONOUS
import dateutil.parser

def getLatencies(subscriberCircuits, secondsToRun):
	interfaces = [interfaceA, interfaceB]
	tcpLatency = 0
	listOfAllDiffs = []
	maxLatencyRecordable = 200
	matchableIPs = []
	for circuit in subscriberCircuits:
		for device in circuit['devices']:
			matchableIPs.append(device['ipv4'])
	
	rttDict = {}
	jitterDict = {}
	#for interface in interfaces:
	command = "./pping -i " + interfaceA + " -s " + str(secondsToRun) + " -m"
	commands = command.split(' ')
	wd = ppingLocation
	tcShowResults = subprocess.run(command, shell=True, cwd=wd,stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.decode('utf-8').splitlines()
	for line in tcShowResults:
		if len(line) > 59:
			rtt1 = float(line[18:27])*1000
			rtt2 = float(line[27:36]) *1000
			toAndFrom = line[38:].split(' ')[3]
			fromIP = toAndFrom.split('+')[0].split(':')[0]
			toIP = toAndFrom.split('+')[1].split(':')[0]
			matchedIP = ''
			if fromIP in matchableIPs:
				matchedIP = fromIP
			elif toIP in matchableIPs:
				matchedIP = toIP
			jitter = rtt1 - rtt2
			#Cap ceil
			if rtt1 >= maxLatencyRecordable:
				rtt1 = 200
			#Lowest observed rtt
			if matchedIP in rttDict:
				if rtt1 < rttDict[matchedIP]:
					rttDict[matchedIP] = rtt1
					jitterDict[matchedIP] = jitter
			else:
				rttDict[matchedIP] = rtt1
				jitterDict[matchedIP] = jitter
	for circuit in subscriberCircuits:
		for device in circuit['devices']:
			diffsForThisDevice = []
			if device['ipv4'] in rttDict:
				device['tcpLatency'] = rttDict[device['ipv4']]
			else:
				device['tcpLatency'] = None
			if device['ipv4'] in jitterDict:
				device['tcpJitter'] = jitterDict[device['ipv4']]
			else:
				device['tcpJitter'] = None
	return subscriberCircuits

def getParentNodeStats(parentNodes, subscriberCircuits):
	for parentNode in parentNodes:
		acceptableLatencies = []
		for circuit in subscriberCircuits:
			for device in circuit['devices']:
				if device['ParentNode'] == parentNode['parentNodeName']:
					if device['tcpLatency'] != None:
						acceptableLatencies.append(device['tcpLatency'])
		
		if len(acceptableLatencies) > 0:
			parentNode['tcpLatency'] = sum(acceptableLatencies)/len(acceptableLatencies)
		else:
			parentNode['tcpLatency'] = None
	return parentNodes

def refreshLatencyGraphs(secondsToRun):
	startTime = datetime.now()
	with open('statsByParentNode.json', 'r') as j:
		parentNodes = json.loads(j.read())
	
	with open('statsByCircuit.json', 'r') as j:
		subscriberCircuits = json.loads(j.read())
	
	print("Retrieving circuit statistics")
	subscriberCircuits = getLatencies(subscriberCircuits, secondsToRun)
	
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
	
	for circuit in subscriberCircuits:
		for device in circuit['devices']:
			if device['tcpLatency'] != None:
				p = Point('Latency').tag("Device", device['deviceName']).tag("ParentNode", device['ParentNode']).tag("Type", "Device").field("TCP Latency", device['tcpLatency'])
				queriesToSend.append(p)

	for parentNode in parentNodes:
		if parentNode['tcpLatency'] != None:
			p = Point('Latency').tag("Device", parentNode['parentNodeName']).tag("ParentNode", parentNode['parentNodeName']).tag("Type", "Parent Node").field("TCP Latency", parentNode['tcpLatency'])
			queriesToSend.append(p)
			
	write_api.write(bucket=bucket, record=queriesToSend)
	print("Added " + str(len(queriesToSend)) + " points to InfluxDB.")
	client.close()
	
	#with open('statsByParentNode.json', 'w') as infile:
	#	json.dump(parentNodes, infile)
	
	#with open('statsByDevice.json', 'w') as infile:
	#	json.dump(devices, infile)
	
	endTime = datetime.now()
	durationSeconds = round((endTime - startTime).total_seconds())
	print("Graphs updated within " + str(durationSeconds) + " seconds.")
	
if __name__ == '__main__':
	refreshLatencyGraphs(10)
