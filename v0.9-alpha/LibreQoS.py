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
	
	#Create MQ
	cpusAvailable = 0
	
	path = '/sys/class/net/' + interfaceA + '/queues/'
	directory_contents = os.listdir(path)
	print(directory_contents)
	for item in directory_contents:
		if "tx-" in str(item):
			cpusAvailable += 1
	thisInterface = interfaceA
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for cpu in range(cpusAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(cpu+1) + ' handle ' + str(cpu+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(cpu+1) + ': classid ' + str(cpu+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':1 ' + fqOrCAKE)
		#Default class - traffic gets passed through this limiter if not otherwise classified by the Shaper.csv
		shell('tc class add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':1 classid ' + str(cpu+1) + ':2 htb rate ' + str(defaultClassCapacityDownloadMbps) + 'mbit ceil ' + str(defaultClassCapacityDownloadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':2 ' + fqOrCAKE)
	
	thisInterface = interfaceB
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq')
	for cpu in range(cpusAvailable):
		shell('tc qdisc add dev ' + thisInterface + ' parent 7FFF:' + str(cpu+1) + ' handle ' + str(cpu+1) + ': htb default 2')
		shell('tc class add dev ' + thisInterface + ' parent ' + str(cpu+1) + ': classid ' + str(cpu+1) + ':1 htb rate '+ str(upstreamBandwidthCapacityDownloadMbps) + 'mbit ceil ' + str(upstreamBandwidthCapacityDownloadMbps) + 'mbit')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':1 ' + fqOrCAKE)
		#Default class - traffic gets passed through this limiter if not otherwise classified by the Shaper.csv
		shell('tc class add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':1 classid ' + str(cpu+1) + ':2 htb rate ' + str(defaultClassCapacityDownloadMbps) + 'mbit ceil ' + str(defaultClassCapacityDownloadMbps) + 'mbit prio 5')
		shell('tc qdisc add dev ' + thisInterface + ' parent ' + str(cpu+1) + ':2 ' + fqOrCAKE)
	
	currentCPUcounter = 1
	ipv4Filters = []		
	ipv6Filters = []
	
	cpuMinorCounterDict = {}
	
	for cpu in range(cpusAvailable):
		cpuMinorCounterDict[cpu] = 3
		
	for AP in devicesByAP:
		#Create HTBs by AP
		currentAPname = AP[0]['AP']
		thisAPdownload = accessPointDownloadMbps[currentAPname]
		thisAPupload = accessPointUploadMbps[currentAPname]
		
		major = currentCPUcounter
		minor = cpuMinorCounterDict[currentCPUcounter]
		#HTBs for each AP
		thisHTBclassID = str(currentCPUcounter) + ':' + str(minor)
		# Guarentee AP gets at least 1/2 of its radio capacity, allow up to its max radio capacity when network not at peak load
		shell('tc class add dev ' + interfaceA + ' parent ' + str(currentCPUcounter) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(thisAPdownload/2)) + 'mbit ceil '+ str(round(thisAPdownload)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(currentCPUcounter) + ':' + str(minor) + ' ' + fqOrCAKE)
		shell('tc class add dev ' + interfaceB + ' parent ' + str(major) + ':1 classid ' + str(minor) + ' htb rate '+ str(round(thisAPupload/2)) + 'mbit ceil '+ str(round(thisAPupload)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
		minor += 1
		for device in AP:
			#QDiscs for each AP
			downloadMin = device['downloadMin']
			downloadMax = device['downloadMax']
			uploadMin = device['uploadMin']
			uploadMax = device['uploadMax']
			shell('tc class add dev ' + interfaceA + ' parent ' + thisHTBclassID + ' classid ' + str(minor) + ' htb rate '+ str(downloadMin) + 'mbit ceil '+ str(downloadMax) + 'mbit prio 3') 
			shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
			shell('tc class add dev ' + interfaceB + ' parent ' + thisHTBclassID + ' classid ' + str(minor) + ' htb rate '+ str(uploadMin) + 'mbit ceil '+ str(uploadMax) + 'mbit prio 3') 
			shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(major) + ':' + str(minor) + ' ' + fqOrCAKE)
			if device['ipv4']:
				parentString = str(major) + ':'
				flowIDstring = str(major) + ':' + str(minor)
				ipv4Filters.append((device['ipv4'], parentString, flowIDstring, currentCPUcounter))
			if device['ipv6']:
				parentString = str(major) + ':'
				flowIDstring = str(major) + ':' + str(minor)
				ipv6Filters.append((device['ipv6'], parentString, flowIDstring, currentCPUcounter))
			deviceQDiscID = str(major) + ':' + str(minor)
			device['qdisc'] = str(major) + ':' + str(minor)
			minor += 1
		cpuMinorCounterDict[currentCPUcounter] = minor
		
		currentCPUcounter += 1
		if currentCPUcounter > cpusAvailable:
			currentCPUcounter = 1

	#IPv4 Filters
	hashTableCounter = 3  + cpusAvailable
	for cpu in range(cpusAvailable):
		for ipv4Filter in ipv4Filters:
			ipv4, parent, classid, filterCpuNum = ipv4Filter
			if filterCpuNum is cpu:
				#if '/' in ipv4:
				#	ipv4 = ipv4.split('/')[0]
				filterHandle = hex(filterHandleCounter)
				shell('./xdp-cpumap-tc/src/xdp_iphash_to_cpu_cmdline --add --ip ' + ipv4 + ' --cpu ' + str(filterCpuNum-1) + ' --classid ' + classid)
				filterHandleCounter += 1 
		filterHandleCounter += 1
	
	#Save devices to file to allow for statistics runs
	with open('devices.json', 'w') as outfile:
		json.dump(devices, outfile)
	
	#Done
	currentTimeString = datetime.now().strftime("%d/%m/%Y %H:%M:%S")
	print("Successful run completed on " + currentTimeString)

if __name__ == '__main__':
	refreshShapers()
	print("Program complete")
