# v1.1 alpha

import random
import logging
import os
import io
import json
import csv
import subprocess
from subprocess import PIPE
import ipaddress
from ipaddress import IPv4Address, IPv6Address
import time
from datetime import date, datetime
from ispConfig import fqOrCAKE, upstreamBandwidthCapacityDownloadMbps, upstreamBandwidthCapacityUploadMbps, defaultClassCapacityDownloadMbps, defaultClassCapacityUploadMbps, interfaceA, interfaceB, shapeBySite, enableActualShellCommands, runShellCommandsAsSudo
import collections

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

	# Load Devices
	devices = []
	with open('Shaper.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			deviceID, AP, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
			ipv4 = ipv4.strip()
			ipv6 = ipv6.strip()
			if AP == "":
				AP = "none"
			AP = AP.strip()
			thisDevice = {
			  "id": deviceID,
			  "mac": mac,
			  "AP": AP,
			  "hostname": hostname,
			  "ipv4": ipv4,
			  "ipv6": ipv6,
			  "downloadMin": round(int(downloadMin)*tcpOverheadFactor),
			  "uploadMin": round(int(uploadMin)*tcpOverheadFactor),
			  "downloadMax": round(int(downloadMax)*tcpOverheadFactor),
			  "uploadMax": round(int(uploadMax)*tcpOverheadFactor),
			  "qdisc": '',
			}
			devices.append(thisDevice)
	
	#Load network heirarchy
	with open('network.json', 'r') as j:
		network = json.loads(j.read())
		
	#Clear Prior Settings
	clearPriorSettings(interfaceA, interfaceB)

	# Find queues available
	queuesAvailable = 0
	path = '/sys/class/net/' + interfaceA + '/queues/'
	directory_contents = os.listdir(path)
	print(directory_contents)
	for item in directory_contents:
		if "tx-" in str(item):
			queuesAvailable += 1
			
	# For VMs, must reduce queues if more than 9, for some reason
	if queuesAvailable > 9:
		command = 'grep -q ^flags.*\ hypervisor\  /proc/cpuinfo && echo "This machine is a VM"'
		try:
			output = subprocess.check_output(command, stderr=subprocess.STDOUT, shell=True).decode()
			success = True 
		except subprocess.CalledProcessError as e:
			output = e.output.decode()
			success = False
		if "This machine is a VM" in output:
			queuesAvailable = 9

	# XDP-CPUMAP-TC
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceA + ' --default --disable')
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceB + ' --default --disable')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --clear')
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceA)
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceB)

	# Create MQ
	thisInterface = interfaceA
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(queue+1) + ' handle ' + str(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ': classid ' + str(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 ' + fqOrCAKE)
		# Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv
		# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
		# Default class can use up to defaultClassCapacityDownloadMbps when that bandwidth isn't used by known hosts.
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 classid ' + str(queue+1) + ':2 htb rate ' + str(defaultClassCapacityDownloadMbps/4) + 'mbit ceil ' + str(defaultClassCapacityDownloadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':2 ' + fqOrCAKE)
	
	thisInterface = interfaceB
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(queue+1) + ' handle ' + str(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ': classid ' + str(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 ' + fqOrCAKE)
		# Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv.
		# Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
		# Default class can use up to defaultClassCapacityUploadMbps when that bandwidth isn't used by known hosts.
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 classid ' + str(queue+1) + ':2 htb rate ' + str(defaultClassCapacityUploadMbps/4) + 'mbit ceil ' + str(defaultClassCapacityUploadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':2 ' + fqOrCAKE)
	print()

	#Establish queue counter
	currentQueueCounter = 1
	queueMinorCounterDict = {}
	# :1 and :2 are used for root and default classes, so start each queue's counter at :3
	for queueNum in range(queuesAvailable):
		queueMinorCounterDict[queueNum+1] = 3

	#Parse network.json. For each tier, create corresponding HTB and leaf classes
	for tier1 in network:
		tabs = ''
		major = currentQueueCounter
		minor = queueMinorCounterDict[currentQueueCounter]
		tier1classID = str(currentQueueCounter) + ':' + str(minor)
		print(tier1)
		tier1download = network[tier1]['downloadBandwidthMbps']
		tier1upload = network[tier1]['uploadBandwidthMbps']
		print("Download:  " + str(tier1download) + " Mbps")
		print("Upload:    " + str(tier1upload) + " Mbps")
		shell('tc class add dev ' + interfaceA + ' parent ' + str(major) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(tier1download/4)) + 'mbit ceil '+ str(round(tier1download)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
		shell('tc class add dev ' + interfaceB + ' parent ' + str(major) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(tier1upload/4)) + 'mbit ceil '+ str(round(tier1upload)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
		print()
		minor += 1
		for device in devices:
			if tier1 == device['AP']:
				maxDownload = min(device['downloadMax'],tier1download)
				maxUpload = min(device['uploadMax'],tier1upload)
				minDownload = device['downloadMin']
				minUpload = device['uploadMin']
				print(tabs + '\t' + device['hostname'])
				print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
				print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
				print(tabs + '\t', end='')
				shell('tc class add dev ' + interfaceA + ' parent ' + tier1classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
				print(tabs + '\t', end='')
				shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
				print(tabs + '\t', end='')
				shell('tc class add dev ' + interfaceB + ' parent ' + tier1classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
				print(tabs + '\t', end='')
				shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
				if device['ipv4']:
					parentString = str(major) + ':'
					flowIDstring = str(major) + ':' + str(minor)
					print(tabs + '\t', end='')
					shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
				print()
				minor += 1
		minor += 1
		if 'children' in network[tier1]:
			for tier2 in network[tier1]['children']:
				tier2classID = str(currentQueueCounter) + ':' + str(minor)
				tabs = '\t'
				print(tabs + tier2)
				tier2download = min(network[tier1]['children'][tier2]['downloadBandwidthMbps'],tier1download)
				tier2upload = min(network[tier1]['children'][tier2]['uploadBandwidthMbps'],tier1upload)
				print(tabs + "Download:  " + str(tier2download) + " Mbps")
				print(tabs + "Upload:    " + str(tier2upload) + " Mbps")
				print(tabs, end='')
				shell('tc class add dev ' + interfaceA + ' parent ' + tier1classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier2download/4)) + 'mbit ceil '+ str(round(tier2download)) + 'mbit prio 3')
				print(tabs, end='')
				shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
				print(tabs, end='')
				shell('tc class add dev ' + interfaceB + ' parent ' + tier1classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier2upload/4)) + 'mbit ceil '+ str(round(tier2upload)) + 'mbit prio 3') 
				print(tabs, end='')
				shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
				print()
				minor += 1
				for device in devices:
					if tier2 == device['AP']:
						maxDownload = min(device['downloadMax'],tier2download)
						maxUpload = min(device['uploadMax'],tier2upload)
						minDownload = device['downloadMin']
						minUpload = device['uploadMin']
						print(tabs + '\t' + device['hostname'])
						print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
						print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
						print(tabs + '\t', end='')
						shell('tc class add dev ' + interfaceA + ' parent ' + tier2classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
						print(tabs + '\t', end='')
						shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
						print(tabs + '\t', end='')
						shell('tc class add dev ' + interfaceB + ' parent ' + tier2classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
						print(tabs + '\t', end='')
						shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
						if device['ipv4']:
							parentString = str(major) + ':'
							flowIDstring = str(major) + ':' + str(minor)
							print(tabs + '\t', end='')
							shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
						print()
						minor += 1
				minor += 1
				if 'children' in network[tier1]['children'][tier2]:
					for tier3 in network[tier1]['children'][tier2]['children']:
						tier3classID = str(currentQueueCounter) + ':' + str(minor)
						tabs = '\t\t'
						print(tabs + tier3)
						tier3download = min(network[tier1]['children'][tier2]['children'][tier3]['downloadBandwidthMbps'],tier2download)
						tier3upload = min(network[tier1]['children'][tier2]['children'][tier3]['uploadBandwidthMbps'],tier2upload)
						print(tabs + "Download:  " + str(tier3download) + " Mbps")
						print(tabs + "Upload:    " + str(tier3upload) + " Mbps")
						print(tabs, end='')
						shell('tc class add dev ' + interfaceA + ' parent ' + tier2classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier3download/4)) + 'mbit ceil '+ str(round(tier3download)) + 'mbit prio 3')
						print(tabs, end='')
						shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
						print(tabs, end='')
						shell('tc class add dev ' + interfaceB + ' parent ' + tier2classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier3upload/4)) + 'mbit ceil '+ str(round(tier3upload)) + 'mbit prio 3') 
						print(tabs, end='')
						shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
						print()
						minor += 1
						for device in devices:
							if tier3 == device['AP']:
								maxDownload = min(device['downloadMax'],tier3download)
								maxUpload = min(device['uploadMax'],tier3upload)
								minDownload = device['downloadMin']
								minUpload = device['uploadMin']
								print(tabs + '\t' + device['hostname'])
								print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
								print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
								print(tabs + '\t', end='')
								shell('tc class add dev ' + interfaceA + ' parent ' + tier3classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
								print(tabs + '\t', end='')
								shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
								print(tabs + '\t', end='')
								shell('tc class add dev ' + interfaceB + ' parent ' + tier3classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
								print(tabs + '\t', end='')
								shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
								if device['ipv4']:
									parentString = str(major) + ':'
									flowIDstring = str(major) + ':' + str(minor)
									print(tabs + '\t', end='')
									shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
								print()
								minor += 1
						minor += 1
						if 'children' in network[tier1]['children'][tier2]['children'][tier3]:
							for tier4 in network[tier1]['children'][tier2]['children'][tier3]['children']:
								tier4classID = str(currentQueueCounter) + ':' + str(minor)
								tabs = '\t\t\t'
								print(tabs + tier4)
								tier4download = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['downloadBandwidthMbps'],tier3download)
								tier4upload = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['uploadBandwidthMbps'],tier3upload)
								print(tabs + "Download:  " + str(tier4download) + " Mbps")
								print(tabs + "Upload:    " + str(tier4upload) + " Mbps")
								print(tabs, end='')
								shell('tc class add dev ' + interfaceA + ' parent ' + tier3classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier4download/4)) + 'mbit ceil '+ str(round(tier4download)) + 'mbit prio 3')
								print(tabs, end='')
								shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
								print(tabs, end='')
								shell('tc class add dev ' + interfaceB + ' parent ' + tier3classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier4upload/4)) + 'mbit ceil '+ str(round(tier4upload)) + 'mbit prio 3') 
								print(tabs, end='')
								shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
								print()
								minor += 1
								for device in devices:
									if tier4 == device['AP']:
										maxDownload = min(device['downloadMax'],tier4download)
										maxUpload = min(device['uploadMax'],tier4upload)
										minDownload = device['downloadMin']
										minUpload = device['uploadMin']
										print(tabs + '\t' + device['hostname'])
										print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
										print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
										print(tabs + '\t', end='')
										shell('tc class add dev ' + interfaceA + ' parent ' + tier4classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
										print(tabs + '\t', end='')
										shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
										print(tabs + '\t', end='')
										shell('tc class add dev ' + interfaceB + ' parent ' + tier4classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
										print(tabs + '\t', end='')
										shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
										if device['ipv4']:
											parentString = str(major) + ':'
											flowIDstring = str(major) + ':' + str(minor)
											print(tabs + '\t', end='')
											shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
										print()
										minor += 1
								minor += 1
								if 'children' in network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]:
									for tier5 in network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children']:
										tier5classID = str(currentQueueCounter) + ':' + str(minor)
										tabs = '\t\t\t\t'
										print(tabs + tier5)
										tier5download = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['downloadBandwidthMbps'],tier4download)
										tier5upload = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['uploadBandwidthMbps'],tier4upload)
										print(tabs + "Download:  " + str(tier5download) + " Mbps")
										print(tabs + "Upload:    " + str(tier5upload) + " Mbps")
										print(tabs, end='')
										shell('tc class add dev ' + interfaceA + ' parent ' + tier4classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier5download/4)) + 'mbit ceil '+ str(round(tier5download)) + 'mbit prio 3')
										print(tabs, end='')
										shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
										print(tabs, end='')
										shell('tc class add dev ' + interfaceB + ' parent ' + tier4classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier5upload/4)) + 'mbit ceil '+ str(round(tier5upload)) + 'mbit prio 3') 
										print(tabs, end='')
										shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
										print()
										minor += 1
										for device in devices:
											if tier5 == device['AP']:
												maxDownload = min(device['downloadMax'],tier5download)
												maxUpload = min(device['uploadMax'],tier5upload)
												minDownload = device['downloadMin']
												minUpload = device['uploadMin']
												minMaxString = "Min: " + str(minDownload) + '/' + str(minUpload) + " Mbps | Max: " + str(maxDownload) + '/' + str(maxUpload) + " Mbps"
												print(tabs + '\t' + device['hostname'])
												print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
												print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
												print(tabs + '\t', end='')
												shell('tc class add dev ' + interfaceA + ' parent ' + tier5classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
												print(tabs + '\t', end='')
												shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
												print(tabs + '\t', end='')
												shell('tc class add dev ' + interfaceB + ' parent ' + tier5classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
												print(tabs + '\t', end='')
												shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
												if device['ipv4']:
													parentString = str(major) + ':'
													flowIDstring = str(major) + ':' + str(minor)
													print(tabs + '\t', end='')
													shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
												print()
												minor += 1
										minor += 1
										if 'children' in network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]:
											for tier6 in network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children']:
												tier6classID = str(currentQueueCounter) + ':' + str(minor)
												tabs = '\t\t\t\t\t'
												print(tabs + tier6)
												tier6download = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children'][tier6]['downloadBandwidthMbps'],tier5download)
												tier6upload = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children'][tier6]['uploadBandwidthMbps'],tier5upload)
												print(tabs + "Download:  " + str(tier6download) + " Mbps")
												print(tabs + "Upload:    " + str(tier6upload) + " Mbps")
												print(tabs, end='')
												shell('tc class add dev ' + interfaceA + ' parent ' + tier5classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier6download/4)) + 'mbit ceil '+ str(round(tier6download)) + 'mbit prio 3')
												print(tabs, end='')
												shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
												print(tabs, end='')
												shell('tc class add dev ' + interfaceB + ' parent ' + tier5classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier6upload/4)) + 'mbit ceil '+ str(round(tier6upload)) + 'mbit prio 3') 
												print(tabs, end='')
												shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
												print()
												minor += 1
												for device in devices:
													if tier6 == device['AP']:
														maxDownload = min(device['downloadMax'],tier6download)
														maxUpload = min(device['uploadMax'],tier6upload)
														minDownload = device['downloadMin']
														minUpload = device['uploadMin']
														print(tabs + '\t' + device['hostname'])
														print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
														print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
														print(tabs + '\t', end='')
														shell('tc class add dev ' + interfaceA + ' parent ' + tier6classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
														print(tabs + '\t', end='')
														shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
														print(tabs + '\t', end='')
														shell('tc class add dev ' + interfaceB + ' parent ' + tier6classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
														print(tabs + '\t', end='')
														shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
														if device['ipv4']:
															parentString = str(major) + ':'
															flowIDstring = str(major) + ':' + str(minor)
															print(tabs + '\t', end='')
															shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
														print()
														minor += 1
												minor += 1
												if 'children' in tier6:
													for tier7 in network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children'][tier6]['children']:
														tier7classID = str(currentQueueCounter) + ':' + str(minor)
														tabs = '\t\t\t\t\t\t'
														print(tabs + tier7)
														tier7download = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children'][tier6]['children'][tier7]['downloadBandwidthMbps'],tier6download)
														tier7upload = min(network[tier1]['children'][tier2]['children'][tier3]['children'][tier4]['children'][tier5]['children'][tier6]['children'][tier7]['uploadBandwidthMbps'],tier6upload)
														print(tabs + "Download:  " + str(tier7download) + " Mbps")
														print(tabs + "Upload:    " + str(tier7upload) + " Mbps")
														print(tabs, end='')
														shell('tc class add dev ' + interfaceA + ' parent ' + tier6classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier7download/4)) + 'mbit ceil '+ str(round(tier7download)) + 'mbit prio 3')
														print(tabs, end='')
														shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
														print(tabs, end='')
														shell('tc class add dev ' + interfaceB + ' parent ' + tier6classID + ' classid ' + str(minor) + ' htb rate '+ str(round(tier7upload/4)) + 'mbit ceil '+ str(round(tier7upload)) + 'mbit prio 3') 
														print(tabs, end='')
														shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
														print()
														minor += 1
														for device in devices:
															if tier7 == device['AP']:
																maxDownload = min(device['downloadMax'],tier7download)
																maxUpload = min(device['uploadMax'],tier7upload)
																minDownload = device['downloadMin']
																minUpload = device['uploadMin']
																print(tabs + '\t' + device['hostname'])
																print(tabs + '\t' + "Download:  " + str(minDownload) + " to " + str(maxDownload) + " Mbps")
																print(tabs + '\t' + "Upload:    " + str(minUpload) + " to " + str(maxUpload) + " Mbps")
																print(tabs + '\t', end='')
																shell('tc class add dev ' + interfaceA + ' parent ' + tier7classID + ' classid ' + str(minor) + ' htb rate '+ str(minDownload) + 'mbit ceil '+ str(maxDownload) + 'mbit prio 3')
																print(tabs + '\t', end='')
																shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
																print(tabs + '\t', end='')
																shell('tc class add dev ' + interfaceB + ' parent ' + tier7classID + ' classid ' + str(minor) + ' htb rate '+ str(minUpload) + 'mbit ceil '+ str(maxUpload) + 'mbit prio 3') 
																print(tabs + '\t', end='')
																shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
																if device['ipv4']:
																	parentString = str(major) + ':'
																	flowIDstring = str(major) + ':' + str(minor)
																	print(tabs + '\t', end='')
																	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
																print()
																minor += 1
														minor += 1
														if 'children' in tier7:
															raise ValueError('File network.json has more than 7 levels of heirarchy. Cannot parse.')	
		queueMinorCounterDict[currentQueueCounter] = minor
		currentQueueCounter += 1
		if currentQueueCounter > queuesAvailable:
			currentQueueCounter = 1
	
	# Done
	currentTimeString = datetime.now().strftime("%d/%m/%Y %H:%M:%S")
	print("Successful run completed on " + currentTimeString)

if __name__ == '__main__':
	refreshShapers()
	print("Program complete")
