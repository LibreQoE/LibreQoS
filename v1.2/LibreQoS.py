#!/usr/bin/python3
# v1.2 alpha

import csv
import io
import ipaddress
import json
import os
import os.path
import subprocess
from subprocess import PIPE, STDOUT
from datetime import datetime, timedelta
import multiprocessing
import warnings
import psutil
import argparse
import logging
import shutil
import binpacking

from ispConfig import fqOrCAKE, upstreamBandwidthCapacityDownloadMbps, upstreamBandwidthCapacityUploadMbps, \
	interfaceA, interfaceB, enableActualShellCommands, \
	runShellCommandsAsSudo, generatedPNDownloadMbps, generatedPNUploadMbps, usingXDP, queuesAvailable

def shell(command):
	if enableActualShellCommands:
		if runShellCommandsAsSudo:
			command = 'sudo ' + command
		logging.info(command)
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			logging.info(line)
			if ("RTNETLINK answers" in line) or ("We have an error talking to the kernel" in line):
				warnings.warn("Command: '" + command + "' resulted in " + line, stacklevel=2)
	else:
		logging.info(command)

def checkIfFirstRunSinceBoot():
	if os.path.isfile("lastRun.txt"):
		with open("lastRun.txt", 'r') as file:
			lastRun = datetime.strptime(file.read(), "%d-%b-%Y (%H:%M:%S.%f)")
		systemRunningSince = datetime.fromtimestamp(psutil.boot_time())
		if systemRunningSince > lastRun:
			print("First time run since system boot.")
			return True
		else:
			print("Not first time run since system boot.")
			return False
	else:
		print("First time run since system boot.")
		return True
	
def clearPriorSettings(interfaceA, interfaceB):
	if enableActualShellCommands:
		# If not using XDP, clear tc filter
		if usingXDP == False:
			#two of these are probably redundant. Will remove later once determined which those are.
			shell('tc filter delete dev ' + interfaceA)
			shell('tc filter delete dev ' + interfaceA + ' root')
			shell('tc filter delete dev ' + interfaceB)
			shell('tc filter delete dev ' + interfaceB + ' root')
		shell('tc qdisc delete dev ' + interfaceA + ' root')
		shell('tc qdisc delete dev ' + interfaceB + ' root')
		#shell('tc qdisc delete dev ' + interfaceA)
		#shell('tc qdisc delete dev ' + interfaceB)
		
def tearDown(interfaceA, interfaceB):
	# Full teardown of everything for exiting LibreQoS
	if enableActualShellCommands:
		# If using XDP, clear IP filters and remove xdp program from interfaces
		if usingXDP == True:
			result = os.system('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --clear')
			shell('ip link set dev ' + interfaceA + ' xdp off')
			shell('ip link set dev ' + interfaceB + ' xdp off')
		clearPriorSettings(interfaceA, interfaceB)

def findQueuesAvailable():
	# Find queues and CPU cores available. Use min between those two as queuesAvailable
	if enableActualShellCommands:
		#queuesAvailable = 0
		#path = '/sys/class/net/' + interfaceA + '/queues/'
		#directory_contents = os.listdir(path)
		#for item in directory_contents:
		#	if "tx-" in str(item):
		#		queuesAvailable += 1
		#print("NIC queues:\t\t\t" + str(queuesAvailable))
		#cpuCount = multiprocessing.cpu_count()
		#print("CPU cores:\t\t\t" + str(cpuCount))
		#queuesAvailable = min(queuesAvailable,cpuCount)
		print("queuesAvailable set to:\t" + str(queuesAvailable))
	else:
		print("As enableActualShellCommands is False, CPU core / queue count has been set to 16")
		logging.info("NIC queues:\t\t\t" + str(16))
		cpuCount = multiprocessing.cpu_count()
		logging.info("CPU cores:\t\t\t" + str(16))
		logging.info("queuesAvailable set to:\t" + str(16))
		queuesAvailable = 16
	return queuesAvailable

def validateNetworkAndDevices():
	# Verify Network.json is valid json
	networkValidatedOrNot = True
	with open('network.json') as file:
		try:
			temporaryVariable = json.load(file) # put JSON-data to a variable
		except json.decoder.JSONDecodeError:
			warnings.warn("network.json is an invalid JSON file", stacklevel=2) # in case json is invalid
			networkValidatedOrNot = False
	if networkValidatedOrNot == True:
		print("network.json passed validation") 
	# Verify ShapedDevices.csv is valid
	devicesValidatedOrNot = True # True by default, switches to false if ANY entry in ShapedDevices.csv fails validation
	rowNum = 2
	with open('ShapedDevices.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		#Remove comments if any
		commentsRemoved = []
		for row in csv_reader:
			if not row[0].startswith('#'):
				commentsRemoved.append(row)
		#Remove header
		commentsRemoved.pop(0) 
		seenTheseIPsAlready = []
		for row in commentsRemoved:
			circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4_input, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment = row
			# Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s seperated by commas. Split them up and parse each to ensure valid
			ipv4_hosts = []
			ipv6_subnets_and_hosts = []
			if ipv4_input != "":
				try:
					ipv4_input = ipv4_input.replace(' ','')
					if "," in ipv4_input:
						ipv4_list = ipv4_input.split(',')
					else:
						ipv4_list = [ipv4_input]
					for ipEntry in ipv4_list:
						if ipEntry in seenTheseIPsAlready:
							warnings.warn("Provided IPv4 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is duplicate.", stacklevel=2)
							devicesValidatedOrNot = False
							seenTheseIPsAlready.append(ipEntry)
						else:
							if '/32' in ipEntry:
								ipEntry = ipEntry.replace('/32','')
								ipv4_hosts.append(ipaddress.ip_address(ipEntry))
							elif '/' in ipEntry:
								ipv4_hosts.extend(list(ipaddress.ip_network(ipEntry).hosts()))
							else:
								ipv4_hosts.append(ipaddress.ip_address(ipEntry))
							seenTheseIPsAlready.append(ipEntry)
				except:
						warnings.warn("Provided IPv4 '" + ipv4_input + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
						devicesValidatedOrNot = False
			if ipv6_input != "":
				try:
					ipv6_input = ipv6_input.replace(' ','')
					if "," in ipv6_input:
						ipv6_list = ipv6_input.split(',')
					else:
						ipv6_list = [ipv6_input]
					for ipEntry in ipv6_list:
						if ipEntry in seenTheseIPsAlready:
							warnings.warn("Provided IPv6 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is duplicate.", stacklevel=2)
							devicesValidatedOrNot = False
							seenTheseIPsAlready.append(ipEntry)
						else:
							if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv6Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv6Address):
								ipv6_subnets_and_hosts.extend(ipEntry)
							else:
								warnings.warn("Provided IPv6 '" + ipv6_input + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
								devicesValidatedOrNot = False
							seenTheseIPsAlready.append(ipEntry)
				except:
						warnings.warn("Provided IPv6 '" + ipv6_input + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
						devicesValidatedOrNot = False
			try:
				a = int(downloadMin)
				if a < 1:
					warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 1 Mbps.", stacklevel=2)
					devicesValidatedOrNot = False
			except:
				warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid integer.", stacklevel=2)
				devicesValidatedOrNot = False
			try:
				a = int(uploadMin)
				if a < 1:
					warnings.warn("Provided uploadMin '" + uploadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 1 Mbps.", stacklevel=2)
					devicesValidatedOrNot = False
			except:
				warnings.warn("Provided uploadMin '" + uploadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid integer.", stacklevel=2)
				devicesValidatedOrNot = False
			try:
				a = int(downloadMax)
				if a < 2:
					warnings.warn("Provided downloadMax '" + downloadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 2 Mbps.", stacklevel=2)
					devicesValidatedOrNot = False
			except:
				warnings.warn("Provided downloadMax '" + downloadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid integer.", stacklevel=2)
				devicesValidatedOrNot = False
			try:
				a = int(uploadMax)
				if a < 2:
					warnings.warn("Provided uploadMax '" + uploadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 2 Mbps.", stacklevel=2)
					devicesValidatedOrNot = False
			except:
				warnings.warn("Provided uploadMax '" + uploadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid integer.", stacklevel=2)
				devicesValidatedOrNot = False
			
			try:
				if int(downloadMin) > int(downloadMax):
					warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is greater than downloadMax", stacklevel=2)
					devicesValidatedOrNot = False
				if int(uploadMin) > int(uploadMax):
					warnings.warn("Provided uploadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is greater than uploadMax", stacklevel=2)
					devicesValidatedOrNot = False
			except:
				devicesValidatedOrNot = False
			
			rowNum += 1
	if devicesValidatedOrNot == True:
		print("ShapedDevices.csv passed validation")
	else:
		print("ShapedDevices.csv failed validation")
	
	if (devicesValidatedOrNot == True) and (devicesValidatedOrNot == True):
		return True
	else:
		return False

def refreshShapers():
	
	# Starting
	print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))
	
	
	# Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
	if enableActualShellCommands == False:
		warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)
	
	
	# Check if first run since boot
	isThisFirstRunSinceBoot = checkIfFirstRunSinceBoot()
	
	
	# Automatically account for TCP overhead of plans. For example a 100Mbps plan needs to be set to 109Mbps for the user to ever see that result on a speed test
	# Does not apply to nodes of any sort, just endpoint devices
	tcpOverheadFactor = 1.09
	
	
	# Files
	shapedDevicesFile = 'ShapedDevices.csv'
	networkJSONfile = 'network.json'
	
	
	# Check validation
	safeToRunRefresh = False
	if (validateNetworkAndDevices() == True):
		shutil.copyfile('ShapedDevices.csv', 'lastGoodConfig.csv')
		shutil.copyfile('network.json', 'lastGoodConfig.json')
		print("Backed up good config as lastGoodConfig.csv and lastGoodConfig.json")
		safeToRunRefresh = True
	else:
		if (isThisFirstRunSinceBoot == False):
			warnings.warn("Validation failed. Because this is not the first run since boot (queues already set up) - will now exit.", stacklevel=2)
			safeToRunRefresh = False
		else:
			warnings.warn("Validation failed. However - because this is the first run since boot - will load queues from last good config", stacklevel=2)
			shapedDevicesFile = 'lastGoodConfig.csv'
			networkJSONfile = 'lastGoodConfig.json'
			safeToRunRefresh = True
	
	if safeToRunRefresh == True:
		
		# Load Subscriber Circuits & Devices
		subscriberCircuits = []
		knownCircuitIDs = []
		counterForCircuitsWithoutParentNodes = 0
		dictForCircuitsWithoutParentNodes = {}
		with open(shapedDevicesFile) as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			# Remove comments if any
			commentsRemoved = []
			for row in csv_reader:
				if not row[0].startswith('#'):
					commentsRemoved.append(row)
			# Remove header
			commentsRemoved.pop(0)
			for row in commentsRemoved:
				circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4_input, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment = row
				ipv4_hosts = []
				# Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s seperated by commas. Split them up and parse each
				if ipv4_input != "":
					ipv4_input = ipv4_input.replace(' ','')
					if "," in ipv4_input:
						ipv4_list = ipv4_input.split(',')
					else:
						ipv4_list = [ipv4_input]
					for ipEntry in ipv4_list:
						if '/32' in ipEntry:
							ipv4_hosts.append(ipEntry.replace('/32',''))
						elif '/' in ipEntry:
							theseHosts = ipaddress.ip_network(ipEntry).hosts()
							for host in theseHosts:
								host = str(host)
								if '/32' in host:
									host = host.replace('/32','')
								ipv4_hosts.append(host)
						else:
							ipv4_hosts.append(ipEntry)
				ipv6_subnets_and_hosts = []
				if ipv6_input != "":
					ipv6_input = ipv6_input.replace(' ','')
					if "," in ipv6_input:
						ipv6_list = ipv6_input.split(',')
					else:
						ipv6_list = [ipv6_input]
					for ipEntry in ipv6_list:
						ipv6_subnets_and_hosts.append(ipEntry)
				# If there is something in the circuit ID field
				if circuitID != "":
					# Seen circuit before
					if circuitID in knownCircuitIDs:
						for circuit in subscriberCircuits:
							if circuit['circuitID'] == circuitID:
								if circuit['ParentNode'] != "none":
									if circuit['ParentNode'] != ParentNode:
										errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Parent Node from other devices of circuit ID #" + circuitID
										raise ValueError(errorMessageString)
								if ((circuit['downloadMin'] != round(int(downloadMin)*tcpOverheadFactor))
									or (circuit['uploadMin'] != round(int(uploadMin)*tcpOverheadFactor))
									or (circuit['downloadMax'] != round(int(downloadMax)*tcpOverheadFactor))
									or (circuit['uploadMax'] != round(int(uploadMax)*tcpOverheadFactor))):
									warnings.warn("Device " + deviceName + " with ID " + deviceID + " had different bandwidth parameters than other devices on this circuit. Will instead use the bandwidth parameters defined by the first device added to its circuit.", stacklevel=2)
								devicesListForCircuit = circuit['devices']
								thisDevice = 	{
												  "deviceID": deviceID,
												  "deviceName": deviceName,
												  "mac": mac,
												  "ipv4s": ipv4_hosts,
												  "ipv6s": ipv6_subnets_and_hosts,
												  "comment": comment
												}
								devicesListForCircuit.append(thisDevice)
								circuit['devices'] = devicesListForCircuit
					# Have not seen circuit before
					else:
						knownCircuitIDs.append(circuitID)
						if ParentNode == "":
							ParentNode = "none"
						ParentNode = ParentNode.strip()
						deviceListForCircuit = []
						thisDevice = 	{
										  "deviceID": deviceID,
										  "deviceName": deviceName,
										  "mac": mac,
										  "ipv4s": ipv4_hosts,
										  "ipv6s": ipv6_subnets_and_hosts,
										  "comment": comment
										}
						deviceListForCircuit.append(thisDevice)
						thisCircuit = {
						  "circuitID": circuitID,
						  "circuitName": circuitName,
						  "ParentNode": ParentNode,
						  "devices": deviceListForCircuit,
						  "downloadMin": round(int(downloadMin)*tcpOverheadFactor),
						  "uploadMin": round(int(uploadMin)*tcpOverheadFactor),
						  "downloadMax": round(int(downloadMax)*tcpOverheadFactor),
						  "uploadMax": round(int(uploadMax)*tcpOverheadFactor),
						  "qdisc": '',
						  "comment": comment
						}
						if thisCircuit['ParentNode'] == 'none':
							thisCircuit['idForCircuitsWithoutParentNodes'] = counterForCircuitsWithoutParentNodes
							dictForCircuitsWithoutParentNodes[counterForCircuitsWithoutParentNodes] = ((round(int(downloadMax)*tcpOverheadFactor))+(round(int(uploadMax)*tcpOverheadFactor))) 
							counterForCircuitsWithoutParentNodes += 1
						subscriberCircuits.append(thisCircuit)
				# If there is nothing in the circuit ID field
				else:
					# Copy deviceName to circuitName if none defined already
					if circuitName == "":
						circuitName = deviceName
					if ParentNode == "":
						ParentNode = "none"
					ParentNode = ParentNode.strip()
					deviceListForCircuit = []
					thisDevice = 	{
									  "deviceID": deviceID,
									  "deviceName": deviceName,
									  "mac": mac,
									  "ipv4s": ipv4_hosts,
									  "ipv6s": ipv6_subnets_and_hosts,
									}
					deviceListForCircuit.append(thisDevice)
					thisCircuit = {
					  "circuitID": circuitID,
					  "circuitName": circuitName,
					  "ParentNode": ParentNode,
					  "devices": deviceListForCircuit,
					  "downloadMin": round(int(downloadMin)*tcpOverheadFactor),
					  "uploadMin": round(int(uploadMin)*tcpOverheadFactor),
					  "downloadMax": round(int(downloadMax)*tcpOverheadFactor),
					  "uploadMax": round(int(uploadMax)*tcpOverheadFactor),
					  "qdisc": '',
					  "comment": comment
					}
					if thisCircuit['ParentNode'] == 'none':
						thisCircuit['idForCircuitsWithoutParentNodes'] = counterForCircuitsWithoutParentNodes
						dictForCircuitsWithoutParentNodes[counterForCircuitsWithoutParentNodes] = ((round(int(downloadMax)*tcpOverheadFactor))+(round(int(uploadMax)*tcpOverheadFactor)))
						counterForCircuitsWithoutParentNodes += 1
					subscriberCircuits.append(thisCircuit)


		# Load network heirarchy
		with open(networkJSONfile, 'r') as j:
			network = json.loads(j.read())
		
		
		# Pull rx/tx queues / CPU cores available
		if usingXDP:
			queuesAvailable = findQueuesAvailable()
			#Determine how many CPU cores will be used for XDP. If graphingEnabled and infludDBisLocal, pin InfluxDB to last core
			#if (graphingEnabled) and (infludDBisLocal):
				#lastCPUnum = queuesAvailable-1
				#XDP mapping and queueing will be limited to queuesAvailable-1 (last core reserved for InfluxDB)
				#queuesAvailable = queuesAvailable-1
				#Pin influxdb to lastCPUnum
				#shell('taskset -cp 15 1339')
		else:
			queuesAvailable = 1


		# Generate Parent Nodes. Spread ShapedDevices.csv which lack defined ParentNode across these (balance across CPUs)
		generatedPNs = []
		for x in range(queuesAvailable):
			genPNname = "Generated_PN_" + str(x+1)
			network[genPNname] =	{
										"downloadBandwidthMbps":generatedPNDownloadMbps,
										"uploadBandwidthMbps":generatedPNUploadMbps
									}
			generatedPNs.append(genPNname)
		bins = binpacking.to_constant_bin_number(dictForCircuitsWithoutParentNodes, queuesAvailable)
		genPNcounter = 0
		for binItem in bins:
			sumItem = 0
			logging.info(generatedPNs[genPNcounter] + " will contain " + str(len(binItem)) + " circuits")
			for key in binItem.keys():
				for circuit in subscriberCircuits:
					if circuit['ParentNode'] == 'none':
						if circuit['idForCircuitsWithoutParentNodes'] == key:
							circuit['ParentNode'] = generatedPNs[genPNcounter]
			genPNcounter += 1
			if genPNcounter >= queuesAvailable:
				genPNcounter = 0
		
		
		# Find the bandwidth minimums for each node by combining mimimums of devices lower in that node's heirarchy
		def findBandwidthMins(data, depth):
			tabs = '   ' * depth
			minDownload = 0
			minUpload = 0
			for elem in data:
				for circuit in subscriberCircuits:
					if elem == circuit['ParentNode']:
						minDownload += circuit['downloadMin']
						minUpload += circuit['uploadMin']
				if 'children' in data[elem]:
					minDL, minUL = findBandwidthMins(data[elem]['children'], depth+1)
					minDownload += minDL
					minUpload += minUL
				data[elem]['downloadBandwidthMbpsMin'] = minDownload
				data[elem]['uploadBandwidthMbpsMin'] = minUpload
			return minDownload, minUpload
		minDownload, minUpload = findBandwidthMins(network, 0)
		
		
		# Parse network structure and add devices from ShapedDevices.csv
		linuxTCcommands = []
		xdpCPUmapCommands = []
		parentNodes = []
		def traverseNetwork(data, depth, major, minor, queue, parentClassID, parentMaxDL, parentMaxUL):
			for node in data:
				circuitsForThisNetworkNode = []
				nodeClassID = hex(major) + ':' + hex(minor)
				data[node]['classid'] = nodeClassID
				data[node]['parentClassID'] = parentClassID
				# Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
				data[node]['downloadBandwidthMbps'] = min(data[node]['downloadBandwidthMbps'],parentMaxDL)
				data[node]['uploadBandwidthMbps'] = min(data[node]['uploadBandwidthMbps'],parentMaxUL)
				# Calculations are done in findBandwidthMins(), determine optimal HTB rates (mins) and ceils (maxs)
				# For some reason that doesn't always yield the expected result, so it's better to play with ceil more than rate
				# Here we override the rate as 95% of ceil.
				data[node]['downloadBandwidthMbpsMin'] = round(data[node]['downloadBandwidthMbps']*.95)
				data[node]['uploadBandwidthMbpsMin'] = round(data[node]['uploadBandwidthMbps']*.95)
				data[node]['classMajor'] = hex(major)
				data[node]['classMinor'] = hex(minor)
				data[node]['cpuNum'] = hex(queue-1)
				thisParentNode =	{
									"parentNodeName": node,
									"classID": nodeClassID,
									"downloadMax": data[node]['downloadBandwidthMbps'],
									"uploadMax": data[node]['uploadBandwidthMbps'],
									}
				parentNodes.append(thisParentNode)
				minor += 1
				for circuit in subscriberCircuits:
					#If a device from ShapedDevices.csv lists this node as its Parent Node, attach it as a leaf to this node HTB
					if node == circuit['ParentNode']:
						if circuit['downloadMax'] > data[node]['downloadBandwidthMbps']:
							warnings.warn("downloadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
						if circuit['uploadMax'] > data[node]['uploadBandwidthMbps']:
							warnings.warn("uploadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
						parentString = hex(major) + ':'
						flowIDstring = hex(major) + ':' + hex(minor)
						circuit['qdisc'] = flowIDstring
						# Create circuit dictionary to be added to network structure, eventually output as queuingStructure.json
						maxDownload = min(circuit['downloadMax'],data[node]['downloadBandwidthMbps'])
						maxUpload = min(circuit['uploadMax'],data[node]['uploadBandwidthMbps'])
						minDownload = min(circuit['downloadMin'],maxDownload)
						minUpload = min(circuit['uploadMin'],maxUpload)
						thisNewCircuitItemForNetwork = {
							'maxDownload' : maxDownload,
							'maxUpload' : maxUpload,
							'minDownload' : minDownload,
							'minUpload' : minUpload,
							"circuitID": circuit['circuitID'],
							"circuitName": circuit['circuitName'],
							"ParentNode": circuit['ParentNode'],
							"devices": circuit['devices'],
							"qdisc": flowIDstring,
							"classMajor": hex(major),
							"classMinor": hex(minor),
							"comment": circuit['comment']
						}
						# Generate TC commands to be executed later
						thisNewCircuitItemForNetwork['devices'] = circuit['devices']
						circuitsForThisNetworkNode.append(thisNewCircuitItemForNetwork)
						minor += 1
				if len(circuitsForThisNetworkNode) > 0:
					data[node]['circuits'] = circuitsForThisNetworkNode
				# Recursive call this function for children nodes attached to this node
				if 'children' in data[node]:
					# We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
					minor = traverseNetwork(data[node]['children'], depth+1, major, minor+1, queue, nodeClassID, data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'])
				# If top level node, increment to next queue / cpu core
				if depth == 0:
					if queue >= queuesAvailable:
						queue = 1
						major = queue
					else:
						queue += 1
						major += 1
			return minor
		# Here is the actual call to the recursive traverseNetwork() function. finalMinor is not used.
		finalMinor = traverseNetwork(network, 0, major=1, minor=3, queue=1, parentClassID="1:1", parentMaxDL=upstreamBandwidthCapacityDownloadMbps, parentMaxUL=upstreamBandwidthCapacityUploadMbps)
		
		
		linuxTCcommands = []
		xdpCPUmapCommands = []
		devicesShaped = []
		# Root HTB Setup
		# If using XDP, Setup MQ
		if usingXDP:
			# Create MQ qdisc for each CPU core / rx-tx queue (XDP method - requires IPv4)
			thisInterface = interfaceA
			logging.info("# MQ Setup for " + thisInterface)
			command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
			linuxTCcommands.append(command)
			for queue in range(queuesAvailable):
				command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
				linuxTCcommands.append(command)
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE
				linuxTCcommands.append(command)
				# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
				# Technically, that should not even happen. So don't expect much if any traffic in this default class.
				# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityDownloadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps-1) + 'mbit prio 5'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE
				linuxTCcommands.append(command)
			
			thisInterface = interfaceB
			logging.info("# MQ Setup for " + thisInterface)
			command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
			linuxTCcommands.append(command)
			for queue in range(queuesAvailable):
				command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
				linuxTCcommands.append(command)
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE
				linuxTCcommands.append(command)
				# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
				# Technically, that should not even happen. So don't expect much if any traffic in this default class.
				# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityUploadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps-1) + 'mbit prio 5'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE
				linuxTCcommands.append(command)
		# If not using XDP, Setup single HTB
		else:
			# Create single HTB qdisc (non XDP method - allows IPv6)
			thisInterface = interfaceA
			command = 'qdisc replace dev ' + thisInterface + ' root handle 0x1: htb default 2 r2q 1514'
			linuxTCcommands.append(command)
			for queue in range(queuesAvailable):
				command = 'qdisc add dev ' + thisInterface + ' parent 0x1:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
				linuxTCcommands.append(command)
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE
				linuxTCcommands.append(command)
				# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
				# Technically, that should not even happen. So don't expect much if any traffic in this default class.
				# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityDownloadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps-1) + 'mbit prio 5'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE
				linuxTCcommands.append(command)
			
			thisInterface = interfaceB
			command = 'qdisc replace dev ' + thisInterface + ' root handle 0x1: htb default 2 r2q 1514'
			linuxTCcommands.append(command)
			for queue in range(queuesAvailable):
				command = 'qdisc add dev ' + thisInterface + ' parent 0x1:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
				linuxTCcommands.append(command)
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE
				linuxTCcommands.append(command)
				# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
				# Technically, that should not even happen. So don't expect much if any traffic in this default class.
				# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
				command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityUploadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps-1) + 'mbit prio 5'
				linuxTCcommands.append(command)
				command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE
				linuxTCcommands.append(command)
		
		
		# Parse network structure. For each tier, generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
		# Define lists for hash filters
		ipv4FiltersSrc = []		
		ipv4FiltersDst = []
		ipv6FiltersSrc = []
		ipv6FiltersDst = []
		def traverseNetwork(data):
			for node in data:
				command = 'class add dev ' + interfaceA + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ str(data[node]['downloadBandwidthMbpsMin']) + 'mbit ceil '+ str(data[node]['downloadBandwidthMbps']) + 'mbit prio 3' + " # Node: " + node
				linuxTCcommands.append(command)
				command = 'class add dev ' + interfaceB + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ str(data[node]['uploadBandwidthMbpsMin']) + 'mbit ceil '+ str(data[node]['uploadBandwidthMbps']) + 'mbit prio 3'
				linuxTCcommands.append(command)
				if 'circuits' in data[node]:
					for circuit in data[node]['circuits']:
						# Generate TC commands to be executed later
						comment = " # CircuitID: " + circuit['circuitID'] + " DeviceIDs: "
						for device in circuit['devices']:
							comment = comment + device['deviceID'] + ', '
						if 'devices' in circuit:
							if 'comment' in circuit['devices'][0]:
								comment = comment + '| Comment: ' + circuit['devices'][0]['comment']
						command = 'class add dev ' + interfaceA + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ str(circuit['minDownload']) + 'mbit ceil '+ str(circuit['maxDownload']) + 'mbit prio 3' + comment
						linuxTCcommands.append(command)
						command = 'qdisc add dev ' + interfaceA + ' parent ' + circuit['classMajor'] + ':' + circuit['classMinor'] + ' ' + fqOrCAKE
						linuxTCcommands.append(command)
						command = 'class add dev ' + interfaceB + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ str(circuit['minUpload']) + 'mbit ceil '+ str(circuit['maxUpload']) + 'mbit prio 3'
						linuxTCcommands.append(command)
						command = 'qdisc add dev ' + interfaceB + ' parent ' + circuit['classMajor'] + ':' + circuit['classMinor'] + ' ' + fqOrCAKE
						linuxTCcommands.append(command)
						for device in circuit['devices']:
							if device['ipv4s']:
								for ipv4 in device['ipv4s']:
									if usingXDP:
										xdpCPUmapCommands.append('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + str(ipv4) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['qdisc'])
									else:
										ipv4FiltersSrc.append((ipv4, circuit['qdisc'].split(':')[0]+':' , circuit['qdisc']))
										ipv4FiltersDst.append((ipv4, circuit['qdisc'].split(':')[0]+':' , circuit['qdisc']))
							if not usingXDP:
								if device['ipv6s']:
									for ipv6 in device['ipv6s']:
										ipv6FiltersSrc.append((ipv6, circuit['qdisc'].split(':')[0]+':' , circuit['qdisc']))
										ipv6FiltersDst.append((ipv6, circuit['qdisc'].split(':')[0]+':' , circuit['qdisc']))
							if device['deviceName'] not in devicesShaped:
								devicesShaped.append(device['deviceName'])
				# Recursive call this function for children nodes attached to this node
				if 'children' in data[node]:
					traverseNetwork(data[node]['children'])
		# Here is the actual call to the recursive traverseNetwork() function. finalResult is not used.
		traverseNetwork(network)
		
		
		# Save queuingStructure
		with open('queuingStructure.json', 'w') as infile:
			json.dump(network, infile, indent=4)
		

		# If XDP off - prepare commands for Hash Tables
		# IPv4 Hash Filters
		# Dst
		command = 'filter add dev ' + interfaceA + ' parent 0x1: protocol all u32'
		linuxTCcommands.append(command)
		command = 'filter add dev ' + interfaceA + ' parent 0x1: protocol ip handle 3: u32 divisor 256'
		linuxTCcommands.append(command)
		filterHandleCounter = 101
		for i in range (256):
			hexID = str(hex(i))#.replace('0x','')
			for ipv4Filter in ipv4FiltersDst:
				ipv4, parent, classid = ipv4Filter
				if '/' in ipv4:
					ipv4 = ipv4.split('/')[0]
				if (ipv4.split('.', 3)[3]) == str(i):
					filterHandle = hex(filterHandleCounter)
					command = 'filter add dev ' + interfaceA + ' handle ' + filterHandle + ' protocol ip parent 0x1: u32 ht 3:' + hexID + ': match ip dst ' + ipv4 + ' flowid ' + classid
					linuxTCcommands.append(command)
					filterHandleCounter += 1 
		command = 'filter add dev ' + interfaceA + ' protocol ip parent 0x1: u32 ht 800: match ip dst 0.0.0.0/0 hashkey mask 0x000000ff at 16 link 3:'
		linuxTCcommands.append(command)
		# Src
		command = 'filter add dev ' + interfaceB + ' parent 0x1: protocol all u32'
		linuxTCcommands.append(command)
		command = 'filter add dev ' + interfaceB + ' parent 0x1: protocol ip handle 4: u32 divisor 256'
		linuxTCcommands.append(command)
		filterHandleCounter = 101
		for i in range (256):
			hexID = str(hex(i))#.replace('0x','')
			for ipv4Filter in ipv4FiltersSrc:
				ipv4, parent, classid = ipv4Filter
				if '/' in ipv4:
					ipv4 = ipv4.split('/')[0]
				if (ipv4.split('.', 3)[3]) == str(i):
					filterHandle = hex(filterHandleCounter)
					command = 'filter add dev ' + interfaceB + ' handle ' + filterHandle + ' protocol ip parent 0x1: u32 ht 4:' + hexID + ': match ip src ' + ipv4 + ' flowid ' + classid
					linuxTCcommands.append(command)
					filterHandleCounter += 1
		command = 'filter add dev ' + interfaceB + ' protocol ip parent 0x1: u32 ht 800: match ip src 0.0.0.0/0 hashkey mask 0x000000ff at 12 link 4:'
		linuxTCcommands.append(command)
		# IPv6 Hash Filters
		# Dst
		command = 'filter add dev ' + interfaceA + ' parent 0x1: handle 5: protocol ipv6 u32 divisor 256'
		linuxTCcommands.append(command)
		filterHandleCounter = 101
		for ipv6Filter in ipv6FiltersDst:
			ipv6, parent, classid = ipv6Filter
			withoutCIDR = ipv6.split('/')[0]
			third = str(ipaddress.IPv6Address(withoutCIDR).exploded).split(':',5)[3]
			usefulPart = third[:2]
			hexID = usefulPart
			filterHandle = hex(filterHandleCounter)
			command = 'filter add dev ' + interfaceA + ' handle ' + filterHandle + ' protocol ipv6 parent 0x1: u32 ht 5:' + hexID + ': match ip6 dst ' + ipv6 + ' flowid ' + classid
			linuxTCcommands.append(command)
			filterHandleCounter += 1
		filterHandle = hex(filterHandleCounter)
		command = 'filter add dev ' + interfaceA + ' protocol ipv6 parent 0x1: u32 ht 800:: match ip6 dst ::/0 hashkey mask 0x0000ff00 at 28 link 5:'
		linuxTCcommands.append(command)
		filterHandleCounter += 1
		# Src
		command = 'filter add dev ' + interfaceB + ' parent 0x1: handle 6: protocol ipv6 u32 divisor 256'
		linuxTCcommands.append(command)
		filterHandleCounter = 101
		for ipv6Filter in ipv6FiltersSrc:
			ipv6, parent, classid = ipv6Filter
			withoutCIDR = ipv6.split('/')[0]
			third = str(ipaddress.IPv6Address(withoutCIDR).exploded).split(':',5)[3]
			usefulPart = third[:2]
			hexID = usefulPart
			filterHandle = hex(filterHandleCounter)
			command = 'filter add dev ' + interfaceB + ' handle ' + filterHandle + ' protocol ipv6 parent 0x1: u32 ht 6:' + hexID + ': match ip6 src ' + ipv6 + ' flowid ' + classid
			linuxTCcommands.append(command)
			filterHandleCounter += 1
		filterHandle = hex(filterHandleCounter)
		command = 'filter add dev ' + interfaceB + ' protocol ipv6 parent 0x1: u32 ht 800:: match ip6 src ::/0 hashkey mask 0x0000ff00 at 12 link 6:'
		linuxTCcommands.append(command)
		filterHandleCounter += 1
		
		
		# Record start time of actual filter reload
		reloadStartTime = datetime.now()
		
		
		# Clear Prior Settings
		clearPriorSettings(interfaceA, interfaceB)

		
		# If using XDP, Setup XDP and disable XPS regardless of whether it is first run or not (necessary to handle cases where systemctl stop was used)
		xdpStartTime = datetime.now()
		if usingXDP:
			if enableActualShellCommands:
				# Here we use os.system for the command, because otherwise it sometimes gltiches out with Popen in shell()
				result = os.system('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --clear')
			# Set up XDP-CPUMAP-TC
			logging.info("# XDP Setup")
			shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceA + ' --default --disable')
			shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceB + ' --default --disable')
			shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
			shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
			shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceA)
			shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceB)	
		xdpEndTime = datetime.now()
		
		
		# Execute actual Linux TC commands
		tcStartTime = datetime.now()
		print("Executing linux TC class/qdisc commands")
		with open('linux_tc.txt', 'w') as f:
			for command in linuxTCcommands:
				logging.info(command)
				f.write(f"{command}\n")
		if logging.DEBUG <= logging.root.level:
			# Do not --force in debug mode, so we can see any errors 
			shell("/sbin/tc -b linux_tc.txt")
		else:
			shell("/sbin/tc -f -b linux_tc.txt")
		tcEndTime = datetime.now()
		print("Executed " + str(len(linuxTCcommands)) + " linux TC class/qdisc commands")
		
		
		# Execute actual XDP-CPUMAP-TC filter commands
		xdpFilterStartTime = datetime.now()
		if usingXDP:
			print("Executing XDP-CPUMAP-TC IP filter commands")
			if enableActualShellCommands:
				for command in xdpCPUmapCommands:
					logging.info(command)
					commands = command.split(' ')
					proc = subprocess.Popen(commands, stdout=subprocess.DEVNULL)
			else:
				for command in xdpCPUmapCommands:
					logging.info(command)
			print("Executed " + str(len(xdpCPUmapCommands)) + " XDP-CPUMAP-TC IP filter commands")
		xdpFilterEndTime = datetime.now()
		
		
		# Record end time of all reload commands
		reloadEndTime = datetime.now()
		
		
		# Recap - warn operator if devices were skipped
		devicesSkipped = []
		for circuit in subscriberCircuits:
			for device in circuit['devices']:
				if device['deviceName'] not in devicesShaped:
					devicesSkipped.append((device['deviceName'],device['deviceID']))
		if len(devicesSkipped) > 0:
			warnings.warn('Some devices were not shaped. Please check to ensure they have a valid ParentNode listed in ShapedDevices.csv:', stacklevel=2)
			print("Devices not shaped:")
			for entry in devicesSkipped:
				name, idNum = entry
				print('DeviceID: ' + idNum + '\t DeviceName: ' + name)
		
		
		# Save for stats
		with open('statsByCircuit.json', 'w') as f:
			f.write(json.dumps(subscriberCircuits, indent=4))
		with open('statsByParentNode.json', 'w') as f:
			f.write(json.dumps(parentNodes, indent=4))
		
		
		# Record time this run completed at
		# filename = os.path.join(_here, 'lastRun.txt')
		with open("lastRun.txt", 'w') as file:
			file.write(datetime.now().strftime("%d-%b-%Y (%H:%M:%S.%f)"))
		
		
		# Report reload time
		reloadTimeSeconds = ((reloadEndTime - reloadStartTime).seconds) + (((reloadEndTime - reloadStartTime).microseconds) / 1000000)
		tcTimeSeconds = ((tcEndTime - tcStartTime).seconds) + (((tcEndTime - tcStartTime).microseconds) / 1000000)
		xdpSetupTimeSeconds = ((xdpEndTime - xdpStartTime).seconds) + (((xdpEndTime - xdpStartTime).microseconds) / 1000000)
		xdpFilterTimeSeconds = ((xdpFilterEndTime - xdpFilterStartTime).seconds) + (((xdpFilterEndTime - xdpFilterStartTime).microseconds) / 1000000)
		print("Queue and IP filter reload completed in " + "{:g}".format(round(reloadTimeSeconds,1)) + " seconds")
		print("\tTC commands: \t" + "{:g}".format(round(tcTimeSeconds,1)) + " seconds")
		print("\tXDP setup: \t " + "{:g}".format(round(xdpSetupTimeSeconds,1)) + " seconds")
		print("\tXDP filters: \t " + "{:g}".format(round(xdpFilterTimeSeconds,1)) + " seconds")
		
		
		# Done
		print("refreshShapers completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

if __name__ == '__main__':
	parser = argparse.ArgumentParser()
	parser.add_argument(
		'-d', '--debug',
		help="Print lots of debugging statements",
		action="store_const", dest="loglevel", const=logging.DEBUG,
		default=logging.WARNING,
	)
	parser.add_argument(
		'-v', '--verbose',
		help="Be verbose",
		action="store_const", dest="loglevel", const=logging.INFO,
	)
	parser.add_argument(
		'--validate',
		help="Just validate network.json and ShapedDevices.csv",
		action=argparse.BooleanOptionalAction,
	)
	parser.add_argument(
		'--clearrules',
		help="Clear ip filters, qdiscs, and xdp setup if any",
		action=argparse.BooleanOptionalAction,
	)
	args = parser.parse_args()    
	logging.basicConfig(level=args.loglevel)
	
	if args.validate:
		status = validateNetworkAndDevices()
	elif args.clearrules:
		tearDown(interfaceA, interfaceB)
	else:
		# Refresh and/or set up queues
		refreshShapers()
