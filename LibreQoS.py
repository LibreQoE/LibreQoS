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
#                          v.0.76-alpha
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
from ispConfig import fqOrCAKE, pipeBandwidthCapacityMbps, interfaceA, interfaceB, enableActualShellCommands, runShellCommandsAsSudo

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

def clearMemoryCache():
	command = "sudo sh -c 'echo 1 >/proc/sys/vm/drop_caches'"
	commands = command.split(' ')
	proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
	for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):
		print(line)

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
	filterHandleCounter = 101
	#Load Devices
	with open('Shaper.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			deviceID, AP, mac, hostname,ipv4, ipv6, download, upload = row
			ipv4 = ipv4.strip()
			ipv6 = ipv6.strip()
			thisDevice = {
			  "id": deviceID,
			  "mac": mac,
			  "AP": AP,
			  "hostname": hostname,
			  "ipv4": ipv4,
			  "ipv6": ipv6,
			  "download": int(download),
			  "upload": int(upload),
			  "qdiscSrc": '',
			  "qdiscDst": '',
			}
			devices.append(thisDevice)
	
	#Clear Prior Configs
	clearPriorSettings(interfaceA, interfaceB)
	
	shell('tc filter delete dev ' + interfaceA + ' parent 1: u32')
	shell('tc filter delete dev ' + interfaceB + ' parent 2: u32')
	
	ipv4FiltersSrc = []		
	ipv4FiltersDst = []
	ipv6FiltersSrc = []
	ipv6FiltersDst = []
	
	#InterfaceA
	parentIDFirstPart = 1
	srcOrDst = 'dst'
	thisInterface = interfaceA
	classIDCounter = 101
	hashIDCounter = parentIDFirstPart + 1
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 1: htb default 15') 
	shell('tc class add dev ' + thisInterface + ' parent 1: classid 1:1 htb rate '+ str(pipeBandwidthCapacityMbps) + 'mbit')
	shell('tc qdisc add dev ' + thisInterface + ' parent 1:1 ' + fqOrCAKE)
	#Default class - traffic gets passed through this limiter if not otherwise classified by the Shaper.csv
	shell('tc class add dev ' + thisInterface + ' parent 1:1 classid 1:15 htb rate 750mbit ceil 750mbit prio 5')
	shell('tc qdisc add dev ' + thisInterface + ' parent 1:15 ' + fqOrCAKE)
	handleIDSecond = 1
	for device in devices:
		speedcap = 0
		if srcOrDst == 'dst':
			speedcap = device['download']
		elif srcOrDst == 'src':
			 speedcap = device['upload']
		#Create Hash Table
		shell('tc class add dev ' + thisInterface + ' parent 1:1 classid 1:' + str(classIDCounter) + ' htb rate '+ str(speedcap) + 'mbit ceil '+ str(round(speedcap*1.05)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + thisInterface + ' parent 1:' + str(classIDCounter) + ' ' + fqOrCAKE)
		if device['ipv4']:
			parentString = '1:'
			flowIDstring = str(parentIDFirstPart) + ':' + str(classIDCounter)
			ipv4FiltersDst.append((device['ipv4'], parentString, flowIDstring))
		if device['ipv6']:
			parentString = '1:'
			flowIDstring = str(parentIDFirstPart) + ':' + str(classIDCounter)
			ipv6FiltersDst.append((device['ipv6'], parentString, flowIDstring))
		deviceQDiscID = '1:' + str(classIDCounter)
		device['qdiscDst'] = deviceQDiscID
		if srcOrDst == 'src':
			device['qdiscSrc'] = deviceQDiscID
		elif srcOrDst == 'dst':
			device['qdiscDst'] = deviceQDiscID
		classIDCounter += 1
	hashIDCounter += 1
	
	#InterfaceB
	parentIDFirstPart = 2
	srcOrDst = 'src'
	thisInterface = interfaceB
	classIDCounter = 101
	hashIDCounter = parentIDFirstPart + 1
	shell('tc qdisc replace dev ' + thisInterface + ' root handle 2: htb default 15') 
	shell('tc class add dev ' + thisInterface + ' parent 2: classid 2:1 htb rate '+ str(pipeBandwidthCapacityMbps) + 'mbit')
	shell('tc qdisc add dev ' + thisInterface + ' parent 2:1 ' + fqOrCAKE)
	#Default class - traffic gets passed through this limiter if not otherwise classified by the Shaper.csv
	shell('tc class add dev ' + thisInterface + ' parent 2:1 classid 2:15 htb rate 750mbit ceil 750mbit prio 5')
	shell('tc qdisc add dev ' + thisInterface + ' parent 2:15 ' + fqOrCAKE)
	handleIDSecond = 1
	for device in devices:
		speedcap = 0
		if srcOrDst == 'dst':
			speedcap = device['download']
		elif srcOrDst == 'src':
			 speedcap = device['upload']
		#Create Hash Table
		shell('tc class add dev ' + thisInterface + ' parent 2:1 classid 2:' + str(classIDCounter) + ' htb rate '+ str(speedcap) + 'mbit ceil '+ str(round(speedcap*1.05)) + 'mbit prio 3') 
		shell('tc qdisc add dev ' + thisInterface + ' parent 2:' + str(classIDCounter) + ' ' + fqOrCAKE)
		if device['ipv4']:
			parentString = '2:'
			flowIDstring = str(parentIDFirstPart) + ':' + str(classIDCounter)
			ipv4FiltersSrc.append((device['ipv4'], parentString, flowIDstring))
		if device['ipv6']:
			parentString = '2:'
			flowIDstring = str(parentIDFirstPart) + ':' + str(classIDCounter)
			ipv6FiltersSrc.append((device['ipv6'], parentString, flowIDstring))
		deviceQDiscID = '2:' + str(classIDCounter)
		device['qdiscSrc'] = deviceQDiscID
		if srcOrDst == 'src':
			device['qdiscSrc'] = deviceQDiscID
		elif srcOrDst == 'dst':
			device['qdiscDst'] = deviceQDiscID
		classIDCounter += 1
	hashIDCounter += 1

	shell('tc filter add dev ' + interfaceA + ' parent 1: protocol all u32')
	shell('tc filter add dev ' + interfaceB + ' parent 2: protocol all u32')
	
	#IPv4 Hash Filters
	#Dst
	interface = interfaceA
	shell('tc filter add dev ' + interface + ' parent 1: protocol ip handle 3: u32 divisor 256')

	for i in range (256):
		hexID = str(hex(i)).replace('0x','')
		for ipv4Filter in ipv4FiltersDst:
			ipv4, parent, classid = ipv4Filter
			if '/' in ipv4:
				ipv4 = ipv4.split('/')[0]
			if (ipv4.split('.', 3)[3]) == str(i):
				filterHandle = hex(filterHandleCounter)
				shell('tc filter add dev ' + interface + ' handle ' + filterHandle + ' protocol ip parent 1:1 u32 ht 3:' + hexID + ': match ip dst ' + ipv4 + ' flowid ' + classid)
				filterHandleCounter += 1 
	shell('tc filter add dev ' + interface + ' protocol ip parent 1: u32 ht 800: match ip dst 0.0.0.0/0 hashkey mask 0x000000ff at 16 link 3:')
	
	#Src
	interface = interfaceB
	shell('tc filter add dev ' + interface + ' parent 2: protocol ip handle 4: u32 divisor 256')

	for i in range (256):
		hexID = str(hex(i)).replace('0x','')
		for ipv4Filter in ipv4FiltersSrc:
			ipv4, parent, classid = ipv4Filter
			if '/' in ipv4:
				ipv4 = ipv4.split('/')[0]
			if (ipv4.split('.', 3)[3]) == str(i):
				filterHandle = hex(filterHandleCounter)
				shell('tc filter add dev ' + interface + ' handle ' + filterHandle + ' protocol ip parent 2:1 u32 ht 4:' + hexID + ': match ip src ' + ipv4 + ' flowid ' + classid)
				filterHandleCounter += 1
	shell('tc filter add dev ' + interface + ' protocol ip parent 2: u32 ht 800: match ip src 0.0.0.0/0 hashkey mask 0x000000ff at 12 link 4:')

	#IPv6 Hash Filters
	#Dst
	interface = interfaceA
	shell('tc filter add dev ' + interface + ' parent 1: handle 5: protocol ipv6 u32 divisor 256')

	for ipv6Filter in ipv6FiltersDst:
		ipv6, parent, classid = ipv6Filter
		withoutCIDR = ipv6.split('/')[0]
		third = str(IPv6Address(withoutCIDR).exploded).split(':',5)[3]
		usefulPart = third[:2]
		hexID = usefulPart
		filterHandle = hex(filterHandleCounter)
		shell('tc filter add dev ' + interface + ' handle ' + filterHandle + ' protocol ipv6 parent 1: u32 ht 5:' + hexID + ': match ip6 dst ' + ipv6 + ' flowid ' + classid)
		filterHandleCounter += 1
	filterHandle = hex(filterHandleCounter)
	shell('tc filter add dev ' + interface + ' protocol ipv6 parent 1: u32 ht 800:: match ip6 dst ::/0 hashkey mask 0x0000ff00 at 28 link 5:')
	filterHandleCounter += 1
	
	#Src
	interface = interfaceB
	shell('tc filter add dev ' + interface + ' parent 2: handle 6: protocol ipv6 u32 divisor 256')

	for ipv6Filter in ipv6FiltersSrc:
		ipv6, parent, classid = ipv6Filter
		withoutCIDR = ipv6.split('/')[0]
		third = str(IPv6Address(withoutCIDR).exploded).split(':',5)[3]
		usefulPart = third[:2]
		hexID = usefulPart
		filterHandle = hex(filterHandleCounter)
		shell('tc filter add dev ' + interface + ' handle ' + filterHandle + ' protocol ipv6 parent 2: u32 ht 6:' + hexID + ': match ip6 src ' + ipv6 + ' flowid ' + classid)
		filterHandleCounter += 1
	filterHandle = hex(filterHandleCounter)
	shell('tc filter add dev ' + interface + ' protocol ipv6 parent 2: u32 ht 800:: match ip6 src ::/0 hashkey mask 0x0000ff00 at 12 link 6:')
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
