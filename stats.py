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
#                          v.0.7-alpha
#
import os
import subprocess
from subprocess import PIPE
import io
import json
from operator import itemgetter 

def getStatistics():
	tcShowResults = []
	command = 'tc -s qdisc show'
	commands = command.split(' ')
	proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
	for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
		tcShowResults.append(line)
	allQDiscStats = []
	thisFlow = {}
	thisFlowStats = {}
	withinCorrectChunk = False
	for line in tcShowResults:
		if "qdisc fq_codel" in line:
			thisFlow['qDiscID'] = line.split(' ')[6]
			withinCorrectChunk = True
		elif ("Sent " in line) and withinCorrectChunk:
			items = line.split(' ')
			thisFlowStats['MegabytesSent'] = int(int(items[2]) * 0.000001)
			thisFlowStats['PacketsSent'] = int(items[4])
			thisFlowStats['droppedPackets'] = int(items[7].replace(',',''))
			thisFlowStats['overlimitsPackets'] = int(items[9])
			thisFlowStats['requeuedPackets'] = int(items[11].replace(')',''))
			if thisFlowStats['PacketsSent'] > 0:
				overlimitsFreq = (thisFlowStats['overlimitsPackets']/thisFlowStats['PacketsSent'])
			else:
				overlimitsFreq = -1
		elif ('backlog' in line) and withinCorrectChunk:
			items = line.split(' ')
			thisFlowStats['backlogBytes'] = int(items[2].replace('b',''))
			thisFlowStats['backlogPackets'] = int(items[3].replace('p',''))
			thisFlowStats['requeues'] = int(items[5])
		elif ('maxpacket' in line) and withinCorrectChunk:
			items = line.split(' ')
			thisFlowStats['maxPacket'] = int(items[3])
			thisFlowStats['dropOverlimit'] = int(items[5])
			thisFlowStats['newFlowCount'] = int(items[7])
			thisFlowStats['ecnMark'] = int(items[9])
		elif ("new_flows_len" in line) and withinCorrectChunk:
			items = line.split(' ')
			thisFlowStats['newFlowsLen'] = int(items[3])
			thisFlowStats['oldFlowsLen'] = int(items[5])
			if thisFlowStats['PacketsSent'] == 0:
				thisFlowStats['percentageDropped'] = 0
			else:
				thisFlowStats['percentageDropped'] = thisFlowStats['droppedPackets']/thisFlowStats['PacketsSent']
			withinCorrectChunk = False
			thisFlow['stats'] = thisFlowStats
			allQDiscStats.append(thisFlow)
			thisFlowStats = {}
			thisFlow = {}
	#Load identifiers from json file
	updatedFlowStats = []
	with open('shapableDevices.json', 'r') as infile:
		shapableDevices = json.load(infile)
	for shapableDevice in shapableDevices:
		shapableDeviceQDiscSrc = shapableDevice['identification']['qDiscSrc']
		shapableDeviceQDiscDst = shapableDevice['identification']['qDiscDst']
		for device in allQDiscStats:
			deviceDiscID = device['qDiscID']
			if shapableDeviceQDiscSrc == deviceDiscID:
				name = shapableDevice['identification']['name']
				ipAddr = shapableDevice['identification']['ipAddr']
				srcOrDst = 'src'
				tempDict = {'name': name, 'ipAddr': ipAddr, 'srcOrDst': srcOrDst}
				device['identification'] = tempDict
				updatedFlowStats.append(device)
			if shapableDeviceQDiscDst == deviceFlowID:
				name = shapableDevice['identification']['name']
				ipAddr = shapableDevice['identification']['ipAddr']
				srcOrDst = 'dst'
				tempDict = {'name': name, 'ipAddr': ipAddr, 'srcOrDst': srcOrDst}
				device['identification'] = tempDict
				updatedFlowStats.append(device)
	return updatedFlowStats
			
if __name__ == '__main__':
	allQDiscStats = getStatistics()
	#Customer CPEs with most packet drops
	packetDropsCPEs = []
	sumOfPercentDropped  = 0
	pickTop = 10
	for item in allQDiscStats:
		packetDropsCPEs.append((item['identification']['name'], item['identification']['ipAddr'], item['stats']['percentageDropped'], item['identification']['srcOrDst']))
		sumOfPercentDropped += item['stats']['percentageDropped']
	averageOfPercentDropped = sumOfPercentDropped/len(allQDiscStats)
	res = sorted(packetDropsCPEs, key = itemgetter(2), reverse = True)[:pickTop]
	for item in res:
		name, ipAddr, percentageDropped, srcOrDst = item
		v1 = percentageDropped
		v2 = averageOfPercentDropped
		difference = abs(v1-v2)/((v1+v2)/2)
		downOrUp = ''
		if srcOrDst == 'src':
			downOrUp = ' upload'
		elif srcOrDst == 'dst':
			downOrUp = ' download'
		if name:
			print(name + downOrUp + " has high packet drop rate of {0:.2%}".format(percentageDropped) + ", {0:.0%}".format(difference) + " above the average of {0:.2%}".format(averageOfPercentDropped))
		else:
			print(ipAddr + downOrUp + " has high packet drop rate of {0:.2%}".format(percentageDropped) + ", {0:.0%}".format(difference) + " above the average of {0:.2%}".format(averageOfPercentDropped))
