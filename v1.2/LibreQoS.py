# v1.2 alpha

import csv
import io
import ipaddress
import json
import os
import subprocess
from datetime import datetime
import multiprocessing
import warnings

from ispConfig import fqOrCAKE, upstreamBandwidthCapacityDownloadMbps, upstreamBandwidthCapacityUploadMbps, \
	defaultClassCapacityDownloadMbps, defaultClassCapacityUploadMbps, interfaceA, interfaceB, enableActualShellCommands, \
	runShellCommandsAsSudo


def shell(command):
	if enableActualShellCommands:
		if runShellCommandsAsSudo:
			command = 'sudo ' + command
		commands = command.split(' ')
		print(command)
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			print(line)
	else:
		print(command)
		
def clearPriorSettings(interfaceA, interfaceB):
	if enableActualShellCommands:
		shell('tc filter delete dev ' + interfaceA)
		shell('tc filter delete dev ' + interfaceA + ' root')
		shell('tc qdisc delete dev ' + interfaceA + ' root')
		shell('tc qdisc delete dev ' + interfaceA)
		shell('tc filter delete dev ' + interfaceB)
		shell('tc filter delete dev ' + interfaceB + ' root')
		shell('tc qdisc delete dev ' + interfaceB + ' root')
		shell('tc qdisc delete dev ' + interfaceB)

def refreshShapers():
	tcpOverheadFactor = 1.09

	# Load Subscriber Circuits & Devices
	subscriberCircuits = []
	knownCircuitIDs = []
	with open('ShapedDevices.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
			if circuitID != "":
				if circuitID in knownCircuitIDs:
					for circuit in subscriberCircuits:
						if circuit['circuitID'] == circuitID:
							if circuit['ParentNode'] != ParentNode:
								errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Parent Node from other devices of circuit ID #" + circuitID
								raise ValueError(errorMessageString)
							if (downloadMin != circuit['downloadMin']) or (uploadMin != circuit['uploadMin']) or (downloadMax != circuit['downloadMax'])  or (uploadMax != circuit['uploadMax']):
								warnings.warn("Device " + deviceName + " with ID " + deviceID + " had different bandwidth parameters than other devices on this circuit. Will instead use the bandwidth parameters defined by the first device added to its circuit.")
							devicesListForCircuit = circuit['devices']
							thisDevice = 	{
											  "deviceID": deviceID,
											  "deviceName": deviceName,
											  "mac": mac,
											  "ipv4": ipv4,
											  "ipv6": ipv6,
											}
							devicesListForCircuit.append(thisDevice)
							circuit['devices'] = devicesListForCircuit
				else:
					knownCircuitIDs.append(circuitID)
					ipv4 = ipv4.strip()
					ipv6 = ipv6.strip()
					if ParentNode == "":
						ParentNode = "none"
					ParentNode = ParentNode.strip()
					deviceListForCircuit = []
					thisDevice = 	{
									  "deviceID": deviceID,
									  "deviceName": deviceName,
									  "mac": mac,
									  "ipv4": ipv4,
									  "ipv6": ipv6,
									}
					deviceListForCircuit.append(thisDevice)
					thisCircuit = {
					  "circuitID": circuitID,
					  "circuitName": circuitName,
					  "ParentNode": ParentNode,
					  "devices" = deviceListForCircuit
					  "downloadMin": round(int(downloadMin)*tcpOverheadFactor),
					  "uploadMin": round(int(uploadMin)*tcpOverheadFactor),
					  "downloadMax": round(int(downloadMax)*tcpOverheadFactor),
					  "uploadMax": round(int(uploadMax)*tcpOverheadFactor),
					  "qdisc": '',
					}
					subscriberCircuits.append(thisCircuit)
			else:
				ipv4 = ipv4.strip()
				ipv6 = ipv6.strip()
				if ParentNode == "":
					ParentNode = "none"
				ParentNode = ParentNode.strip()
				deviceListForCircuit = []
				thisDevice = 	{
								  "deviceID": deviceID,
								  "deviceName": deviceName,
								  "mac": mac,
								  "ipv4": ipv4,
								  "ipv6": ipv6,
								}
				deviceListForCircuit.append(thisDevice)
				thisCircuit = {
				  "circuitID": circuitID,
				  "circuitName": circuitName,
				  "ParentNode": ParentNode,
				  "devices" = deviceListForCircuit
				  "downloadMin": round(int(downloadMin)*tcpOverheadFactor),
				  "uploadMin": round(int(uploadMin)*tcpOverheadFactor),
				  "downloadMax": round(int(downloadMax)*tcpOverheadFactor),
				  "uploadMax": round(int(uploadMax)*tcpOverheadFactor),
				  "qdisc": '',
				}
				subscriberCircuits.append(thisCircuit)
	
	#Load network heirarchy
	with open('network.json', 'r') as j:
		network = json.loads(j.read())
	
	#Find the bandwidth minimums for each node by combining mimimums of devices lower in that node's heirarchy
	def findBandwidthMins(data, depth):
		tabs = '   ' * depth
		minDownload = 0
		minUpload = 0
		for elem in data:
			
			for device in devices:
				if elem == device['ParentNode']:
					minDownload += device['downloadMin']
					minUpload += device['uploadMin']
			if 'children' in data[elem]:
				minDL, minUL = findBandwidthMins(data[elem]['children'], depth+1)
				minDownload += minDL
				minUpload += minUL
			data[elem]['downloadBandwidthMbpsMin'] = minDownload
			data[elem]['uploadBandwidthMbpsMin'] = minUpload
		return minDownload, minUpload
	
	minDownload, minUpload = findBandwidthMins(network, 0)

	#Clear Prior Settings
	clearPriorSettings(interfaceA, interfaceB)

	# Find queues and CPU cores available. Use min between those two as queuesAvailable
	queuesAvailable = 0
	path = '/sys/class/net/' + interfaceA + '/queues/'
	directory_contents = os.listdir(path)
	for item in directory_contents:
		if "tx-" in str(item):
			queuesAvailable += 1
	
	print("NIC queues:\t" + str(queuesAvailable))
	cpuCount = multiprocessing.cpu_count()
	print("CPU cores:\t" + str(cpuCount))
	queuesAvailable = min(queuesAvailable,cpuCount)
	
	# XDP-CPUMAP-TC
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceA + ' --default --disable')
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceB + ' --default --disable')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --clear')
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceA)
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceB)

	# Create MQ qdisc for each interface
	thisInterface = interfaceA
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE)
		# Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv
		# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
		# Default class can use up to defaultClassCapacityDownloadMbps when that bandwidth isn't used by known hosts.
		shell('tc class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(defaultClassCapacityDownloadMbps/4) + 'mbit ceil ' + str(defaultClassCapacityDownloadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE)
	
	thisInterface = interfaceB
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + fqOrCAKE)
		# Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv.
		# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
		# Default class can use up to defaultClassCapacityUploadMbps when that bandwidth isn't used by known hosts.
		shell('tc class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(defaultClassCapacityUploadMbps/4) + 'mbit ceil ' + str(defaultClassCapacityUploadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + fqOrCAKE)
	print()

	#Parse network.json. For each tier, create corresponding HTB and leaf classes
	devicesShaped = []
	parentNodes = []
	def traverseNetwork(data, depth, major, minor, queue, parentClassID, parentMaxDL, parentMaxUL):
		tabs = '   ' * depth
		for elem in data:
			print(tabs + elem)
			elemClassID = hex(major) + ':' + hex(minor)
			#Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
			elemDownloadMax = min(data[elem]['downloadBandwidthMbps'],parentMaxDL)
			elemUploadMax = min(data[elem]['uploadBandwidthMbps'],parentMaxUL)
			#Based on calculations done in findBandwidthMins(), determine optimal HTB rates (mins) and ceils (maxs)
			#The max calculation is to avoid 0 values, and the min calculation is to ensure rate is not higher than ceil
			elemDownloadMin = round(elemDownloadMax*.95)
			elemUploadMin = round(elemUploadMax*.95)
			print(tabs + "Download:  " + str(elemDownloadMin) + " to " + str(elemDownloadMax) + " Mbps")
			print(tabs + "Upload:    " + str(elemUploadMin) + " to " + str(elemUploadMax) + " Mbps")
			print(tabs, end='')
			shell('tc class add dev ' + interfaceA + ' parent ' + parentClassID + ' classid ' + hex(minor) + ' htb rate '+ str(round(elemDownloadMin)) + 'mbit ceil '+ str(round(elemDownloadMax)) + 'mbit prio 3') 
			print(tabs, end='')
			shell('tc class add dev ' + interfaceB + ' parent ' + parentClassID + ' classid ' + hex(minor) + ' htb rate '+ str(round(elemUploadMin)) + 'mbit ceil '+ str(round(elemUploadMax)) + 'mbit prio 3') 
			print()
			thisParentNode =	{
								"parentNodeName": elem,
								"classID": elemClassID,
								"downloadMax": elemDownloadMax,
								"uploadMax": elemUploadMax,
								}
			parentNodes.append(thisParentNode)
			minor += 1
			for circuit in subscriberCircuits:
				#If a device from Shaper.csv lists this elem as its Parent Node, attach it as a leaf to this elem HTB
				if elem == device['ParentNode']:
					maxDownload = min(device['downloadMax'],elemDownloadMax)
					maxUpload = min(device['uploadMax'],elemUploadMax)
					minDownload = min(device['downloadMin'],maxDownload)
					minUpload = min(device['uploadMin'],maxUpload)
					print(tabs + '   ' + device['hostname'])
					print(tabs + '   ' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
					print(tabs + '   ' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
					print(tabs + '   ', end='')
					shell('tc class add dev ' + interfaceA + ' parent ' + elemClassID + ' classid ' + hex(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
					print(tabs + '   ', end='')
					shell('tc qdisc add dev ' + interfaceA + ' parent ' + hex(major) + ':' + hex(minor) + ' ' + fqOrCAKE)
					print(tabs + '   ', end='')
					shell('tc class add dev ' + interfaceB + ' parent ' + elemClassID + ' classid ' + hex(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
					print(tabs + '   ', end='')
					shell('tc qdisc add dev ' + interfaceB + ' parent ' + hex(major) + ':' + hex(minor) + ' ' + fqOrCAKE)
					for device in circuit['devices']:
						if device['ipv4']:
							parentString = hex(major) + ':'
							flowIDstring = hex(major) + ':' + hex(minor)
							if '/' in device['ipv4']:
								hosts = list(ipaddress.ip_network(device['ipv4']).hosts())
								for host in hosts:
									print(tabs + '   ', end='')
									shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + str(host) + ' --cpu ' + hex(queue-1) + ' --classid ' + flowIDstring)
							else:
								print(tabs + '   ', end='')
								shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + hex(queue-1) + ' --classid ' + flowIDstring)
							device['qdisc'] = flowIDstring
							if device['deviceName'] not in devicesShaped:
								devicesShaped.append(device['deviceName'])
					print()
					minor += 1
			#Recursive call this function for children nodes attached to this node
			if 'children' in data[elem]:
				#We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
				minor = traverseNetwork(data[elem]['children'], depth+1, major, minor+1, queue, elemClassID, elemDownloadMax, elemUploadMax)
			#If top level node, increment to next queue / cpu core
			if depth == 0:
				if queue >= queuesAvailable:
					queue = 1
					major = queue
				else:
					queue += 1
					major += 1
		return minor
	
	#Here is the actual call to the recursive traverseNetwork() function. finalMinor is not used.
	finalMinor = traverseNetwork(network, 0, major=1, minor=3, queue=1, parentClassID="1:1", parentMaxDL=upstreamBandwidthCapacityDownloadMbps, parentMaxUL=upstreamBandwidthCapacityUploadMbps)
	
	#Recap
	for device in devices:
		if device['deviceName'] not in devicesShaped:
			warnings.warn('Device ' + device['deviceName'] + ' with device ID of ' + device['deviceID'] + ' was not shaped. Please check to ensure its Parent Node is listed in network.json.')
	
	#Save for stats
	with open('statsByCircuit.json', 'w') as infile:
		json.dump(subscriberCircuits, infile)
	with open('statsByParentNode.json', 'w') as infile:
		json.dump(parentNodes, infile)

	# Done
	currentTimeString = datetime.now().strftime("%d/%m/%Y %H:%M:%S")
	print("Successful run completed on " + currentTimeString)

if __name__ == '__main__':
	refreshShapers()
	print("Program complete")
