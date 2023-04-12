#!/usr/bin/python3
from pythonCheck import checkPythonVersion
checkPythonVersion()
import csv
import io
import ipaddress
import json
import math
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
from deepdiff import DeepDiff

from ispConfig import sqm, upstreamBandwidthCapacityDownloadMbps, upstreamBandwidthCapacityUploadMbps, \
	interfaceA, interfaceB, enableActualShellCommands, useBinPackingToBalanceCPU, monitorOnlyMode, \
	runShellCommandsAsSudo, generatedPNDownloadMbps, generatedPNUploadMbps, queuesAvailableOverride, \
	OnAStick

from liblqos_python import is_lqosd_alive, clear_ip_mappings, delete_ip_mapping, validate_shaped_devices, \
	is_libre_already_running, create_lock_file, free_lock_file, add_ip_mapping, BatchedCommands

# Automatically account for TCP overhead of plans. For example a 100Mbps plan needs to be set to 109Mbps for the user to ever see that result on a speed test
# Does not apply to nodes of any sort, just endpoint devices
tcpOverheadFactor = 1.09

def shell(command):
	if enableActualShellCommands:
		if runShellCommandsAsSudo:
			command = 'sudo ' + command
		logging.info(command)
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			if logging.DEBUG <= logging.root.level:
				print(line)
			if ("RTNETLINK answers" in line) or ("We have an error talking to the kernel" in line):
				warnings.warn("Command: '" + command + "' resulted in " + line, stacklevel=2)
	else:
		logging.info(command)

def shellReturn(command):
	returnableString = ''
	if enableActualShellCommands:
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			returnableString = returnableString + line + '\n'
	return returnableString

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
		if 'mq' in shellReturn('tc qdisc show dev ' + interfaceA + ' root'):
			print('MQ detected. Will delete and recreate mq qdisc.')
			# Clear tc filter
			if OnAStick == True:
				shell('tc qdisc delete dev ' + interfaceA + ' root')
			else:
				shell('tc qdisc delete dev ' + interfaceA + ' root')
				shell('tc qdisc delete dev ' + interfaceB + ' root')
		
def tearDown(interfaceA, interfaceB):
	# Full teardown of everything for exiting LibreQoS
	if enableActualShellCommands:
		# Clear IP filters and remove xdp program from interfaces
		#result = os.system('./bin/xdp_iphash_to_cpu_cmdline clear')
		clear_ip_mappings() # Use the bus
		clearPriorSettings(interfaceA, interfaceB)

def findQueuesAvailable():
	# Find queues and CPU cores available. Use min between those two as queuesAvailable
	if enableActualShellCommands:
		if queuesAvailableOverride == 0:
			queuesAvailable = 0
			path = '/sys/class/net/' + interfaceA + '/queues/'
			directory_contents = os.listdir(path)
			for item in directory_contents:
				if "tx-" in str(item):
					queuesAvailable += 1
			print("NIC queues:\t\t\t" + str(queuesAvailable))
		else:
			queuesAvailable = queuesAvailableOverride
			print("NIC queues (Override):\t\t\t" + str(queuesAvailable))
		cpuCount = multiprocessing.cpu_count()
		print("CPU cores:\t\t\t" + str(cpuCount))
		if queuesAvailable < 2:
			raise SystemError('Only 1 NIC rx/tx queue available. You will need to use a NIC with 2 or more rx/tx queues available.')
		if queuesAvailable < 2:
			raise SystemError('Only 1 CPU core available. You will need to use a CPU with 2 or more CPU cores.')
		queuesAvailable = min(queuesAvailable,cpuCount)
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
	# Verify ShapedDevices.csv is valid
	devicesValidatedOrNot = True # True by default, switches to false if ANY entry in ShapedDevices.csv fails validation

	# Verify that the Rust side of things can read the CSV file
	rustValid = validate_shaped_devices()
	if rustValid == "OK":
		print("Rust validated ShapedDevices.csv")
	else:
		warnings.warn("Rust failed to validate ShapedDevices.csv", stacklevel=2)
		warnings.warn(rustValid, stacklevel=2)
		devicesValidatedOrNot = False
	with open('network.json') as file:
		try:
			temporaryVariable = json.load(file) # put JSON-data to a variable
		except json.decoder.JSONDecodeError:
			warnings.warn("network.json is an invalid JSON file", stacklevel=2) # in case json is invalid
			networkValidatedOrNot
	if networkValidatedOrNot == True:
		print("network.json passed validation") 
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
			# Must have circuitID, it's a unique identifier required for stateful changes to queue structure
			if circuitID == '':
				warnings.warn("No Circuit ID provided in ShapedDevices.csv at row " + str(rowNum), stacklevel=2)
				devicesValidatedOrNot = False
			# Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s separated by commas. Split them up and parse each to ensure valid
			ipv4_subnets_and_hosts = []
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
							if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv4Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv4Address):
								ipv4_subnets_and_hosts.extend(ipEntry)
							else:
								warnings.warn("Provided IPv4 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
								devicesValidatedOrNot = False
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
								warnings.warn("Provided IPv6 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
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

def loadSubscriberCircuits(shapedDevicesFile):
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
			# If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
			if monitorOnlyMode == True:
				downloadMin = 10000
				uploadMin = 10000
				downloadMax = 10000
				uploadMax = 10000
			ipv4_subnets_and_hosts = []
			# Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s separated by commas. Split them up and parse each
			if ipv4_input != "":
				ipv4_input = ipv4_input.replace(' ','')
				if "," in ipv4_input:
					ipv4_list = ipv4_input.split(',')
				else:
					ipv4_list = [ipv4_input]
				for ipEntry in ipv4_list:
					ipv4_subnets_and_hosts.append(ipEntry)
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
							# Check if bandwidth parameters match other cdevices of this same circuit ID, but only check if monitorOnlyMode is Off
							if monitorOnlyMode == False:
								if ((circuit['minDownload'] != round(int(downloadMin)*tcpOverheadFactor))
									or (circuit['minUpload'] != round(int(uploadMin)*tcpOverheadFactor))
									or (circuit['maxDownload'] != round(int(downloadMax)*tcpOverheadFactor))
									or (circuit['maxUpload'] != round(int(uploadMax)*tcpOverheadFactor))):
									warnings.warn("Device " + deviceName + " with ID " + deviceID + " had different bandwidth parameters than other devices on this circuit. Will instead use the bandwidth parameters defined by the first device added to its circuit.", stacklevel=2)
							devicesListForCircuit = circuit['devices']
							thisDevice = 	{
											  "deviceID": deviceID,
											  "deviceName": deviceName,
											  "mac": mac,
											  "ipv4s": ipv4_subnets_and_hosts,
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
									  "ipv4s": ipv4_subnets_and_hosts,
									  "ipv6s": ipv6_subnets_and_hosts,
									  "comment": comment
									}
					deviceListForCircuit.append(thisDevice)
					thisCircuit = {
					  "circuitID": circuitID,
					  "circuitName": circuitName,
					  "ParentNode": ParentNode,
					  "devices": deviceListForCircuit,
					  "minDownload": round(int(downloadMin)*tcpOverheadFactor),
					  "minUpload": round(int(uploadMin)*tcpOverheadFactor),
					  "maxDownload": round(int(downloadMax)*tcpOverheadFactor),
					  "maxUpload": round(int(uploadMax)*tcpOverheadFactor),
					  "classid": '',
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
								  "ipv4s": ipv4_subnets_and_hosts,
								  "ipv6s": ipv6_subnets_and_hosts,
								}
				deviceListForCircuit.append(thisDevice)
				thisCircuit = {
				  "circuitID": circuitID,
				  "circuitName": circuitName,
				  "ParentNode": ParentNode,
				  "devices": deviceListForCircuit,
				  "minDownload": round(int(downloadMin)*tcpOverheadFactor),
				  "minUpload": round(int(uploadMin)*tcpOverheadFactor),
				  "maxDownload": round(int(downloadMax)*tcpOverheadFactor),
				  "maxUpload": round(int(uploadMax)*tcpOverheadFactor),
				  "classid": '',
				  "comment": comment
				}
				if thisCircuit['ParentNode'] == 'none':
					thisCircuit['idForCircuitsWithoutParentNodes'] = counterForCircuitsWithoutParentNodes
					dictForCircuitsWithoutParentNodes[counterForCircuitsWithoutParentNodes] = ((round(int(downloadMax)*tcpOverheadFactor))+(round(int(uploadMax)*tcpOverheadFactor)))
					counterForCircuitsWithoutParentNodes += 1
				subscriberCircuits.append(thisCircuit)
	return (subscriberCircuits,	dictForCircuitsWithoutParentNodes)

def refreshShapers():
	
	# Starting
	print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))
	# Create a single batch of xdp update commands to execute together
	ipMapBatch = BatchedCommands()
	
	# Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
	if enableActualShellCommands == False:
		warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)
	# Warn user if monitorOnlyMode is True, because that would mean no actual shaping is happening
	if monitorOnlyMode == True:
		warnings.warn("monitorOnlyMode is set to True. Shaping will not occur.", stacklevel=2)
	
	
	# Check if first run since boot
	isThisFirstRunSinceBoot = checkIfFirstRunSinceBoot()
	
	
	# Files
	shapedDevicesFile = 'ShapedDevices.csv'
	networkJSONfile = 'network.json'
	
	
	# Check validation
	safeToRunRefresh = False
	print("Validating input files '" + shapedDevicesFile + "' and '" + networkJSONfile + "'")
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
		subscriberCircuits,	dictForCircuitsWithoutParentNodes = loadSubscriberCircuits(shapedDevicesFile)
		

		# Load network hierarchy
		with open(networkJSONfile, 'r') as j:
			network = json.loads(j.read())
		
		
		# Pull rx/tx queues / CPU cores available
		queuesAvailable = findQueuesAvailable()
		stickOffset = 0
		if OnAStick:
			print("On-a-stick override dividing queues")
			# The idea here is that download use queues 0 - n/2, upload uses the other half
			queuesAvailable = math.floor(queuesAvailable / 2)
			stickOffset = queuesAvailable		
		
		# If in monitorOnlyMode, override network.json bandwidth rates to where no shaping will actually occur
		if monitorOnlyMode == True:
			def overrideNetworkBandwidths(data):
				for elem in data:
					if 'children' in data[elem]:
						overrideNetworkBandwidths(data[elem]['children'])
					data[elem]['downloadBandwidthMbpsMin'] = 10000
					data[elem]['uploadBandwidthMbpsMin'] = 10000
			overrideNetworkBandwidths(network)
		
		# Generate Parent Nodes. Spread ShapedDevices.csv which lack defined ParentNode across these (balance across CPUs)
		print("Generating parent nodes")
		generatedPNs = []
		numberOfGeneratedPNs = queuesAvailable
		# If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
		if monitorOnlyMode == True:
			chosenDownloadMbps = 10000
			chosenUploadMbps = 10000
		else:
			chosenDownloadMbps = generatedPNDownloadMbps
			chosenUploadMbps = generatedPNDownloadMbps
		for x in range(numberOfGeneratedPNs):
			genPNname = "Generated_PN_" + str(x+1)
			network[genPNname] =	{
										"downloadBandwidthMbps": chosenDownloadMbps,
										"uploadBandwidthMbps": chosenUploadMbps
									}
			generatedPNs.append(genPNname)
		if useBinPackingToBalanceCPU:
			print("Using binpacking module to sort circuits by CPU core")
			bins = binpacking.to_constant_bin_number(dictForCircuitsWithoutParentNodes, numberOfGeneratedPNs)
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
			print("Finished binpacking")
		else:
			genPNcounter = 0
			for circuit in subscriberCircuits:
				if circuit['ParentNode'] == 'none':
					circuit['ParentNode'] = generatedPNs[genPNcounter]
					genPNcounter += 1
					if genPNcounter >= queuesAvailable:
						genPNcounter = 0
		print("Generated parent nodes created")
		
		# Find the bandwidth minimums for each node by combining mimimums of devices lower in that node's hierarchy
		def findBandwidthMins(data, depth):
			tabs = '   ' * depth
			minDownload = 0
			minUpload = 0
			for elem in data:
				for circuit in subscriberCircuits:
					if elem == circuit['ParentNode']:
						minDownload += circuit['minDownload']
						minUpload += circuit['minUpload']
				if 'children' in data[elem]:
					minDL, minUL = findBandwidthMins(data[elem]['children'], depth+1)
					minDownload += minDL
					minUpload += minUL
				data[elem]['downloadBandwidthMbpsMin'] = minDownload
				data[elem]['uploadBandwidthMbpsMin'] = minUpload
			return minDownload, minUpload
		logging.info("Finding the bandwidth minimums for each node")
		minDownload, minUpload = findBandwidthMins(network, 0)
		logging.info("Found the bandwidth minimums for each node")
		
		
		# Child nodes inherit bandwidth maximums of parents. We apply this here to avoid bugs when compression is applied with flattenA().
		def inheritBandwidthMaxes(data, parentMaxDL, parentMaxUL):
			for node in data:
				if isinstance(node, str):
					if (isinstance(data[node], dict)) and (node != 'children'):
						# Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
						data[node]['downloadBandwidthMbps'] = min(int(data[node]['downloadBandwidthMbps']),int(parentMaxDL))
						data[node]['uploadBandwidthMbps'] = min(int(data[node]['uploadBandwidthMbps']),int(parentMaxUL))
						# Recursive call this function for children nodes attached to this node
						if 'children' in data[node]:
							# We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
							inheritBandwidthMaxes(data[node]['children'], data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'])
			#return data
		# Here is the actual call to the recursive function
		inheritBandwidthMaxes(network, parentMaxDL=upstreamBandwidthCapacityDownloadMbps, parentMaxUL=upstreamBandwidthCapacityUploadMbps)

		
		# Compress network.json. HTB only supports 8 levels of HTB depth. Compress to 8 layers if beyond 8.
		def flattenB(data):
			newDict = {}
			for node in data:
				if isinstance(node, str):
					if (isinstance(data[node], dict)) and (node != 'children'):
						newDict[node] = dict(data[node])
						if 'children' in data[node]:
							result = flattenB(data[node]['children'])
							del newDict[node]['children']
							newDict.update(result)
			return newDict
		def flattenA(data, depth):
			newDict = {}
			for node in data:
				if isinstance(node, str):
					if (isinstance(data[node], dict)) and (node != 'children'):
						newDict[node] = dict(data[node])
						if 'children' in data[node]:
							result = flattenA(data[node]['children'], depth+2)
							del newDict[node]['children']
							if depth <= 8:
								newDict[node]['children'] = result
							else:
								flattened = flattenB(data[node]['children'])
								if 'children' in newDict[node]:
									newDict[node]['children'].update(flattened)
								else:
									newDict[node]['children'] = flattened
			return newDict
		network = flattenA(network, 1)
		
		# Parse network structure and add devices from ShapedDevices.csv
		parentNodes = []
		minorByCPUpreloaded = {}
		knownClassIDs = []
		# Track minor counter by CPU. This way we can have > 32000 hosts (htb has u16 limit to minor handle)
		for x in range(queuesAvailable):
			minorByCPUpreloaded[x+1] = 3
		def traverseNetwork(data, depth, major, minorByCPU, queue, parentClassID, upParentClassID, parentMaxDL, parentMaxUL):
			for node in data:
				circuitsForThisNetworkNode = []
				nodeClassID = hex(major) + ':' + hex(minorByCPU[queue])
				upNodeClassID = hex(major+stickOffset) + ':' + hex(minorByCPU[queue])
				data[node]['classid'] = nodeClassID
				data[node]['up_classid'] = upNodeClassID
				if depth == 0:
					parentClassID = hex(major) + ':'
					upParentClassID = hex(major+stickOffset) + ':'
				data[node]['parentClassID'] = parentClassID
				data[node]['up_parentClassID'] = upParentClassID
				# If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
				if monitorOnlyMode == True:
					data[node]['downloadBandwidthMbps'] = 10000
					data[node]['uploadBandwidthMbps'] = 10000
				# If not in monitorOnlyMode
				else:
					# Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
					data[node]['downloadBandwidthMbps'] = min(data[node]['downloadBandwidthMbps'],parentMaxDL)
					data[node]['uploadBandwidthMbps'] = min(data[node]['uploadBandwidthMbps'],parentMaxUL)
				# Calculations are done in findBandwidthMins(), determine optimal HTB rates (mins) and ceils (maxs)
				# For some reason that doesn't always yield the expected result, so it's better to play with ceil more than rate
				# Here we override the rate as 95% of ceil.
				data[node]['downloadBandwidthMbpsMin'] = round(data[node]['downloadBandwidthMbps']*.95)
				data[node]['uploadBandwidthMbpsMin'] = round(data[node]['uploadBandwidthMbps']*.95)
				
				data[node]['classMajor'] = hex(major)
				data[node]['up_classMajor'] = hex(major + stickOffset)
				data[node]['classMinor'] = hex(minorByCPU[queue])
				data[node]['cpuNum'] = hex(queue-1)
				data[node]['up_cpuNum'] = hex(queue-1+stickOffset)
				thisParentNode =	{
									"parentNodeName": node,
									"classID": nodeClassID,
									"maxDownload": data[node]['downloadBandwidthMbps'],
									"maxUpload": data[node]['uploadBandwidthMbps'],
									}
				parentNodes.append(thisParentNode)
				minorByCPU[queue] = minorByCPU[queue] + 1
				for circuit in subscriberCircuits:
					#If a device from ShapedDevices.csv lists this node as its Parent Node, attach it as a leaf to this node HTB
					if node == circuit['ParentNode']:
						if monitorOnlyMode == False:
							if circuit['maxDownload'] > data[node]['downloadBandwidthMbps']:
								logging.info("downloadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
							if circuit['maxUpload'] > data[node]['uploadBandwidthMbps']:
								logging.info("uploadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
						parentString = hex(major) + ':'
						flowIDstring = hex(major) + ':' + hex(minorByCPU[queue])
						upFlowIDstring = hex(major + stickOffset) + ':' + hex(minorByCPU[queue])
						circuit['classid'] = flowIDstring
						circuit['up_classid'] = upFlowIDstring
						logging.info("Added up_classid to circuit: " + circuit['up_classid'])
						# Create circuit dictionary to be added to network structure, eventually output as queuingStructure.json
						maxDownload = min(circuit['maxDownload'],data[node]['downloadBandwidthMbps'])
						maxUpload = min(circuit['maxUpload'],data[node]['uploadBandwidthMbps'])
						minDownload = min(circuit['minDownload'],maxDownload)
						minUpload = min(circuit['minUpload'],maxUpload)
						thisNewCircuitItemForNetwork = {
							'maxDownload' : maxDownload,
							'maxUpload' : maxUpload,
							'minDownload' : minDownload,
							'minUpload' : minUpload,
							"circuitID": circuit['circuitID'],
							"circuitName": circuit['circuitName'],
							"ParentNode": circuit['ParentNode'],
							"devices": circuit['devices'],
							"classid": flowIDstring,
							"up_classid" : upFlowIDstring,
							"classMajor": hex(major),
							"up_classMajor" : hex(major + stickOffset),
							"classMinor": hex(minorByCPU[queue]),
							"comment": circuit['comment']
						}
						# Generate TC commands to be executed later
						thisNewCircuitItemForNetwork['devices'] = circuit['devices']
						circuitsForThisNetworkNode.append(thisNewCircuitItemForNetwork)
						minorByCPU[queue] = minorByCPU[queue] + 1
				if len(circuitsForThisNetworkNode) > 0:
					data[node]['circuits'] = circuitsForThisNetworkNode
				# Recursive call this function for children nodes attached to this node
				if 'children' in data[node]:
					# We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
					minorByCPU[queue] = minorByCPU[queue] + 1
					minorByCPU = traverseNetwork(data[node]['children'], depth+1, major, minorByCPU, queue, nodeClassID, upNodeClassID, data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'])
				# If top level node, increment to next queue / cpu core
				if depth == 0:
					if queue >= queuesAvailable:
						queue = 1
						major = queue
					else:
						queue += 1
						major += 1
			return minorByCPU
		# Here is the actual call to the recursive traverseNetwork() function. finalMinor is not used.
		minorByCPU = traverseNetwork(network, 0, major=1, minorByCPU=minorByCPUpreloaded, queue=1, parentClassID=None, upParentClassID=None, parentMaxDL=upstreamBandwidthCapacityDownloadMbps, parentMaxUL=upstreamBandwidthCapacityUploadMbps)
		
		linuxTCcommands = []
		devicesShaped = []
		# Root HTB Setup
		# Create MQ qdisc for each CPU core / rx-tx queue. Generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
		thisInterface = interfaceA
		logging.info("# MQ Setup for " + thisInterface)
		command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
		linuxTCcommands.append(command)
		for queue in range(queuesAvailable):
			command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
			linuxTCcommands.append(command)
			command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit'
			linuxTCcommands.append(command)
			command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm
			linuxTCcommands.append(command)
			# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
			# Technically, that should not even happen. So don't expect much if any traffic in this default class.
			# Only 1/4 of defaultClassCapacity is guaranteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
			command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityDownloadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps-1) + 'mbit prio 5'
			linuxTCcommands.append(command)
			command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm
			linuxTCcommands.append(command)
		
		# Note the use of stickOffset, and not replacing the root queue if we're on a stick
		thisInterface = interfaceB
		logging.info("# MQ Setup for " + thisInterface)
		if not OnAStick:
			command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
			linuxTCcommands.append(command)
		for queue in range(queuesAvailable):
			command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
			linuxTCcommands.append(command)
			command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit'
			linuxTCcommands.append(command)
			command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm
			linuxTCcommands.append(command)
			# Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
			# Technically, that should not even happen. So don't expect much if any traffic in this default class.
			# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
			command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + str(round((upstreamBandwidthCapacityUploadMbps-1)/4)) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps-1) + 'mbit prio 5'
			linuxTCcommands.append(command)
			command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm
			linuxTCcommands.append(command)
		
		
		# Parse network structure. For each tier, generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
		# Define lists for hash filters
		def traverseNetwork(data):
			for node in data:
				command = 'class add dev ' + interfaceA + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ str(data[node]['downloadBandwidthMbpsMin']) + 'mbit ceil '+ str(data[node]['downloadBandwidthMbps']) + 'mbit prio 3'
				linuxTCcommands.append(command)
				logging.info("Up ParentClassID: " + data[node]['up_parentClassID'])
				logging.info("ClassMinor: " + data[node]['classMinor'])
				command = 'class add dev ' + interfaceB + ' parent ' + data[node]['up_parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ str(data[node]['uploadBandwidthMbpsMin']) + 'mbit ceil '+ str(data[node]['uploadBandwidthMbps']) + 'mbit prio 3'
				linuxTCcommands.append(command)
				if 'circuits' in data[node]:
					for circuit in data[node]['circuits']:
						# Generate TC commands to be executed later
						tcComment = " # CircuitID: " + circuit['circuitID'] + " DeviceIDs: "
						for device in circuit['devices']:
							tcComment = tcComment + device['deviceID'] + ', '
						if 'devices' in circuit:
							if 'comment' in circuit['devices'][0]:
								tcComment = tcComment + '| Comment: ' + circuit['devices'][0]['comment']
						tcComment = tcComment.replace("\n", "")
						command = 'class add dev ' + interfaceA + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ str(circuit['minDownload']) + 'mbit ceil '+ str(circuit['maxDownload']) + 'mbit prio 3' + tcComment
						linuxTCcommands.append(command)
						# Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
						if monitorOnlyMode == False:	
							command = 'qdisc add dev ' + interfaceA + ' parent ' + circuit['classMajor'] + ':' + circuit['classMinor'] + ' ' + sqm
							linuxTCcommands.append(command)
						command = 'class add dev ' + interfaceB + ' parent ' + data[node]['up_classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ str(circuit['minUpload']) + 'mbit ceil '+ str(circuit['maxUpload']) + 'mbit prio 3'
						linuxTCcommands.append(command)
						# Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
						if monitorOnlyMode == False:	
							command = 'qdisc add dev ' + interfaceB + ' parent ' + circuit['up_classMajor'] + ':' + circuit['classMinor'] + ' ' + sqm
							linuxTCcommands.append(command)
							pass
						for device in circuit['devices']:
							if device['ipv4s']:
								for ipv4 in device['ipv4s']:
									ipMapBatch.add_ip_mapping(str(ipv4), circuit['classid'], data[node]['cpuNum'], False)
									#xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
									if OnAStick:
										ipMapBatch.add_ip_mapping(str(ipv4), circuit['up_classid'], data[node]['up_cpuNum'], True)
										#xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
							if device['ipv6s']:
								for ipv6 in device['ipv6s']:
									ipMapBatch.add_ip_mapping(str(ipv6), circuit['classid'], data[node]['cpuNum'], False)
									#xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
									if OnAStick:
										ipMapBatch.add_ip_mapping(str(ipv6), circuit['up_classid'], data[node]['up_cpuNum'], True)
										#xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
							if device['deviceName'] not in devicesShaped:
								devicesShaped.append(device['deviceName'])
				# Recursive call this function for children nodes attached to this node
				if 'children' in data[node]:
					traverseNetwork(data[node]['children'])
		# Here is the actual call to the recursive traverseNetwork() function.
		traverseNetwork(network)
		
		# Save queuingStructure
		queuingStructure = {}
		queuingStructure['Network'] = network
		queuingStructure['lastUsedClassIDCounterByCPU'] = minorByCPU
		queuingStructure['generatedPNs'] = generatedPNs
		with open('queuingStructure.json', 'w') as infile:
			json.dump(queuingStructure, infile, indent=4)
		
		
		# Record start time of actual filter reload
		reloadStartTime = datetime.now()
		
		
		# Clear Prior Settings
		clearPriorSettings(interfaceA, interfaceB)

		
		# Setup XDP and disable XPS regardless of whether it is first run or not (necessary to handle cases where systemctl stop was used)
		xdpStartTime = datetime.now()
		if enableActualShellCommands:
			# Here we use os.system for the command, because otherwise it sometimes gltiches out with Popen in shell()
			#result = os.system('./bin/xdp_iphash_to_cpu_cmdline clear')
			clear_ip_mappings() # Use the bus
		# Set up XDP-CPUMAP-TC
		logging.info("# XDP Setup")
		# Commented out - the daemon does this
		#shell('./cpumap-pping/bin/xps_setup.sh -d ' + interfaceA + ' --default --disable')
		#shell('./cpumap-pping/bin/xps_setup.sh -d ' + interfaceB + ' --default --disable')
		#shell('./cpumap-pping/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
		#shell('./cpumap-pping/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
		#shell('./cpumap-pping/src/tc_classify --dev-egress ' + interfaceA)
		#shell('./cpumap-pping/src/tc_classify --dev-egress ' + interfaceB)	
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
		print("Executing XDP-CPUMAP-TC IP filter commands")
		numXdpCommands = ipMapBatch.length();
		if enableActualShellCommands:
			ipMapBatch.submit()
			#for command in xdpCPUmapCommands:
			#	logging.info(command)
			#	commands = command.split(' ')
			#	proc = subprocess.Popen(commands, stdout=subprocess.DEVNULL)
		else:
			ipMapBatch.log()
			#for command in xdpCPUmapCommands:
			#	logging.info(command)
		print("Executed " + str(numXdpCommands) + " XDP-CPUMAP-TC IP filter commands")
		#print(xdpCPUmapCommands)
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
		
		# Save ShapedDevices.csv as ShapedDevices.lastLoaded.csv
		shutil.copyfile('ShapedDevices.csv', 'ShapedDevices.lastLoaded.csv')
		
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
		print("\tXDP filters: \t " + "{:g}".format(round(xdpFilterTimeSeconds,4)) + " seconds")
		
		
		# Done
		print("refreshShapers completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

def refreshShapersUpdateOnly():
	
	# Check that the host lqosd is running
	if is_lqosd_alive():
		print("lqosd is running")
	else:
		print("ERROR: lqosd is not running. Aborting")
		os._exit(-1)
	
	
	# Starting
	print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))
	
	
	# Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
	if enableActualShellCommands == False:
		warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)
	
	
	# Files
	shapedDevicesFile = 'ShapedDevices.csv'
	networkJSONfile = 'network.json'
	
	
	# Check validation
	safeToRunRefresh = False
	if (validateNetworkAndDevices() == True):
		safeToRunRefresh = True
	else:
		warnings.warn("Validation failed. Will now exit.", stacklevel=2)
	
	if safeToRunRefresh == True:
		networkChanged = False
		devicesChanged = False
		# Check for changes to network.json
		if os.path.isfile('lastGoodConfig.json'):
			with open('lastGoodConfig.json', 'r') as j:
				originalNetwork = json.loads(j.read())
			with open('network.json', 'r') as j:
				newestNetwork = json.loads(j.read())
			ddiff = DeepDiff(originalNetwork, newestNetwork, ignore_order=True)
			if ddiff != {}:
				networkChanged = True
		
		# Check for changes to ShapedDevices.csv
		newlyUpdatedSubscriberCircuits,	newlyUpdatedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits('ShapedDevices.csv')					
		lastLoadedSubscriberCircuits, lastLoadedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits('ShapedDevices.lastLoaded.csv')		
		
		newlyUpdatedSubscriberCircuitsByID = {}
		for circuit in newlyUpdatedSubscriberCircuits:
			circuitid = circuit['circuitID']
			newlyUpdatedSubscriberCircuitsByID[circuitid] = circuit
		
		lastLoadedSubscriberCircuitsByID = {}
		for circuit in lastLoadedSubscriberCircuits:
			circuitid = circuit['circuitID']
			lastLoadedSubscriberCircuitsByID[circuitid] = circuit
		
		for circuitID, circuit in lastLoadedSubscriberCircuitsByID.items():
			if (circuitID in newlyUpdatedSubscriberCircuitsByID) and (circuitID in lastLoadedSubscriberCircuitsByID):
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['maxDownload'] != lastLoadedSubscriberCircuitsByID[circuitID]['maxDownload']:
					devicesChanged = True
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['maxUpload'] != lastLoadedSubscriberCircuitsByID[circuitID]['maxUpload']:
					devicesChanged = True
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['minDownload'] != lastLoadedSubscriberCircuitsByID[circuitID]['minDownload']:
					devicesChanged = True
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['minUpload'] != lastLoadedSubscriberCircuitsByID[circuitID]['minUpload']:
					devicesChanged = True					
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['devices'] != lastLoadedSubscriberCircuitsByID[circuitID]['devices']:
					devicesChanged = True
				if newlyUpdatedSubscriberCircuitsByID[circuitID]['ParentNode'] != lastLoadedSubscriberCircuitsByID[circuitID]['ParentNode']:
					devicesChanged = True
			else:
				devicesChanged = True
		for circuitID, circuit in newlyUpdatedSubscriberCircuitsByID.items():
			if (circuitID not in lastLoadedSubscriberCircuitsByID):
				devicesChanged = True
		
		
		if devicesChanged or networkChanged:
			print('Observed changes to ShapedDevices.csv or network.json. Applying full reload now')
			refreshShapers()
		else:
			print('Observed no changes to ShapedDevices.csv or network.json. Leaving queues as is.')
		
		# Done
		print("refreshShapersUpdateOnly completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

if __name__ == '__main__':
	# Check that the host lqosd is running
	if is_lqosd_alive():
		print("lqosd is running")
	else:
		print("ERROR: lqosd is not running. Aborting")
		os._exit(-1)

	# Check that we aren't running LibreQoS.py more than once at a time
	if is_libre_already_running():
		print("LibreQoS.py is already running in another process. Aborting.")
		os._exit(-1)

	# We've got this far, so create a lock file
	create_lock_file()

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
		'--updateonly',
		help="Only update to reflect changes in ShapedDevices.csv (partial reload)",
		action=argparse.BooleanOptionalAction,
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
	elif args.updateonly:
		# Single-interface updates don't work at all right now.
		if OnAStick:
			print("--updateonly is not supported for single-interface configurations")
			os.exit(-1)
		refreshShapersUpdateOnly()
	else:
		# Refresh and/or set up queues
		refreshShapers()
	
	# Free the lock file
	free_lock_file()
