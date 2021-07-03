# Copyright (C) 2020  Robert Chacón
# This file is part of LibreQoS.
#
# LibreQoS is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 2 of the License, or
# (at your option) any later version.
#
# LibreQoS is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with LibreQoS.  If not, see <http://www.gnu.org/licenses/>.
#
#            _     _ _               ___       ____  
#           | |   (_) |__  _ __ ___ / _ \  ___/ ___| 
#           | |   | | '_ \| '__/ _ \ | | |/ _ \___ \ 
#           | |___| | |_) | | |  __/ |_| | (_) |__) |
#           |_____|_|_.__/|_|  \___|\__\_\\___/____/
#                          v.0.90-alpha
#
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
from ispConfig import fqOrCAKE, upstreamBandwidthCapacityDownloadMbps, upstreamBandwidthCapacityUploadMbps, defaultClassCapacityDownloadMbps, defaultClassCapacityUploadMbps, interfaceA, interfaceB, enableActualShellCommands, runShellCommandsAsSudo
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
	shell('tc filter delete dev ' + interfaceA)
	shell('tc filter delete dev ' + interfaceA + ' root')
	shell('tc qdisc delete dev ' + interfaceA + ' root')
	shell('tc qdisc delete dev ' + interfaceA)
	shell('tc filter delete dev ' + interfaceB)
	shell('tc filter delete dev ' + interfaceB + ' root')
	shell('tc qdisc delete dev ' + interfaceB + ' root')
	shell('tc qdisc delete dev ' + interfaceB)
	if runShellCommandsAsSudo:
		clearMemoryCache()

def refreshShapers():
	devices = []
	accessPointDownloadMbps = {}
	accessPointUploadMbps = {}
	filterHandleCounter = 101
	#Load Access Points
	with open('AccessPoints.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			AP, download, upload = row
			accessPointDownloadMbps[AP] = int(download)
			accessPointUploadMbps[AP] = int(upload)
	#Load Devices
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
			  "downloadMin": int(downloadMin),
			  "uploadMin": int(uploadMin),
			  "downloadMax": int(downloadMax),
			  "uploadMax": int(uploadMax),
			  "qdisc": '',
			}
			# If an AP is specified for a device in Shaper.csv, but AP is not listed in AccessPoints.csv, raise exception
			if (AP != "none") and (AP not in accessPointDownloadMbps):
				raise ValueError('AP ' + AP + ' not listed in AccessPoints.csv')		
			devices.append(thisDevice)			
	# If no AP is specified for a device in Shaper.csv, it is placed under this 'default' AP shaper, set to bandwidth max at edge
	accessPointDownloadMbps['none'] = upstreamBandwidthCapacityDownloadMbps
	accessPointUploadMbps['none'] = upstreamBandwidthCapacityUploadMbps
	#Sort into bins by AP
	result = collections.defaultdict(list)
	for d in devices:
		result[d['AP']].append(d)
	devicesByAP = list(result.values())
	clearPriorSettings(interfaceA, interfaceB)
	#XDP-CPUMAP-TC
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceA + ' --default')
	shell('./xdp-cpumap-tc/bin/xps_setup.sh -d ' + interfaceB + ' --default')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --clear')
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceA)
	shell('./xdp-cpumap-tc/src/tc_classify --dev-egress ' + interfaceB)
	#Find queues available
	queuesAvailable = 0
	path = '/sys/class/net/' + interfaceA + '/queues/'
	directory_contents = os.listdir(path)
	print(directory_contents)
	for item in directory_contents:
		if "tx-" in str(item):
			queuesAvailable += 1
	#For VMs, must reduce queues if more than 9, for some reason
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
	#Create MQ
	thisInterface = interfaceA
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(queue+1) + ' handle ' + str(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ': classid ' + str(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 ' + fqOrCAKE)
		#Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 classid ' + str(queue+1) + ':2 htb rate ' + str(defaultClassCapacityDownloadMbps) + 'mbit ceil ' + str(defaultClassCapacityDownloadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':2 ' + fqOrCAKE)
	
	thisInterface = interfaceB
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for queue in range(queuesAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(queue+1) + ' handle ' + str(queue+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ': classid ' + str(queue+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityUploadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityUploadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 ' + fqOrCAKE)
		#Default class - traffic gets passed through this limiter with lower priority if not otherwise classified by the Shaper.csv
		shell('tc class add dev ' + thisInterface + ' parent ' + str(queue+1) + ':1 classid ' + str(queue+1) + ':2 htb rate ' + str(defaultClassCapacityUploadMbps) + 'mbit ceil ' + str(defaultClassCapacityUploadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(queue+1) + ':2 ' + fqOrCAKE)
	
	currentQueueCounter = 1
	queueMinorCounterDict = {}
	# :1 and :2 are used for root and default classes, so start each counter at :3
	for queueNum in range(queuesAvailable):
		queueMinorCounterDict[queueNum+1] = 3
		
	for AP in devicesByAP:
		#Create HTBs by AP
		currentAPname = AP[0]['AP']
		thisAPdownload = accessPointDownloadMbps[currentAPname]
		thisAPupload = accessPointUploadMbps[currentAPname]
		major = currentQueueCounter
		minor = queueMinorCounterDict[currentQueueCounter]
		#HTBs for each AP
		thisHTBclassID = str(currentQueueCounter) + ':' + str(minor)
		# Guarentee AP gets at least 1/4 of its radio capacity, allow up to its max radio capacity when network not at peak load
		shell('tc class add dev ' + interfaceA + ' parent ' + str(currentQueueCounter) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(thisAPdownload/4)) + 'mbit ceil '+ str(round(thisAPdownload)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(currentQueueCounter) + ':' + str(minor) + ' ' + fqOrCAKE)
		shell('tc class add dev ' + interfaceB + ' parent ' + str(major) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(thisAPupload/4)) + 'mbit ceil '+ str(round(thisAPupload)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
		minor += 1
		for device in AP:
			#QDiscs for each AP
			shell('tc class add dev ' + interfaceA + ' parent ' + thisHTBclassID + ' classid ' + str(minor) + ' htb rate '+ str(device['downloadMin']) + 'mbit ceil '+ str(device['downloadMax']) + 'mbit prio 3') 
			shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
			shell('tc class add dev ' + interfaceB + ' parent ' + thisHTBclassID + ' classid ' + str(minor) + ' htb rate '+ str(device['uploadMin']) + 'mbit ceil '+ str(device['uploadMax']) + 'mbit prio 3') 
			shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
			if device['ipv4']:
				parentString = str(major) + ':'
				flowIDstring = str(major) + ':' + str(minor)
				shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv4'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
			#Once XDP-CPUMAP-TC handles IPv6, this can be added
			#if device['ipv6']:
			#	parentString = str(major) + ':'
			#	flowIDstring = str(major) + ':' + str(minor)
			#	shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + device['ipv6'] + ' --cpu ' + str(currentQueueCounter-1) + ' --classid ' + flowIDstring)
			device['qdisc'] = str(major) + ':' + str(minor)
			minor += 1
		queueMinorCounterDict[currentQueueCounter] = minor
		
		currentQueueCounter += 1
		if currentQueueCounter > queuesAvailable:
			currentQueueCounter = 1
	
	#Save devices to file to allow for statistics runs
	with open('devices.json', 'w') as outfile:
		json.dump(devices, outfile)
	
	#Done
	currentTimeString = datetime.now().strftime("%d/%m/%Y %H:%M:%S")
	print("Successful run completed on " + currentTimeString)

if __name__ == '__main__':
	refreshShapers()
	print("Program complete")
