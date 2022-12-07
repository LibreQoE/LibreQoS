import os
import subprocess
from subprocess import PIPE
import io
import decimal
import json
from operator import itemgetter 
from prettytable import PrettyTable
from ispConfig import fqOrCAKE

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
		expecting = "qdisc " + fqOrCAKE
		if expecting in line:
			thisFlow['qDiscID'] = line.split(' ')[6]
			withinCorrectChunk = True
		elif ("Sent " in line) and withinCorrectChunk:
			items = line.split(' ')
			thisFlowStats['GigabytesSent'] = str(round((int(items[2]) * 0.000000001), 1))
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
	#Load shapableDevices
	updatedFlowStats = []
	with open('devices.json', 'r') as infile:
		devices = json.load(infile)
	for shapableDevice in devices:
		shapableDeviceqdiscSrc = shapableDevice['qdiscSrc']
		shapableDeviceqdiscDst = shapableDevice['qdiscDst']
		for device in allQDiscStats:
			deviceFlowID = device['qDiscID']
			if shapableDeviceqdiscSrc == deviceFlowID:
				name = shapableDevice['hostname']
				AP = shapableDevice['AP']
				ipv4 = shapableDevice['ipv4']
				ipv6 = shapableDevice['ipv6']
				srcOrDst = 'src'
				tempDict = {'name': name, 'AP': AP, 'ipv4': ipv4, 'ipv6': ipv6, 'srcOrDst': srcOrDst}
				device['identification'] = tempDict
				updatedFlowStats.append(device)
			if shapableDeviceqdiscDst == deviceFlowID:
				name = shapableDevice['hostname']
				AP = shapableDevice['AP']
				ipv4 = shapableDevice['ipv4']
				ipv6 = shapableDevice['ipv6']
				srcOrDst = 'dst'
				tempDict = {'name': name, 'AP': AP, 'ipv4': ipv4, 'ipv6': ipv6, 'srcOrDst': srcOrDst}
				device['identification'] = tempDict
				updatedFlowStats.append(device)
	mergedStats = []
	for item in updatedFlowStats:
		if item['identification']['srcOrDst'] == 'src':
			newStat = {
				'identification': {
					'name': item['identification']['name'],
					'AP': item['identification']['AP'],
					'ipv4': item['identification']['ipv4'],
					'ipv6': item['identification']['ipv6']
				},
				'src': {
					'GigabytesSent': item['stats']['GigabytesSent'],
					'PacketsSent': item['stats']['PacketsSent'],
					'droppedPackets': item['stats']['droppedPackets'],
					'overlimitsPackets': item['stats']['overlimitsPackets'],
					'requeuedPackets': item['stats']['requeuedPackets'],
					'backlogBytes': item['stats']['backlogBytes'],
					'backlogPackets': item['stats']['backlogPackets'],
					'requeues': item['stats']['requeues'],
					'maxPacket': item['stats']['maxPacket'],
					'dropOverlimit': item['stats']['dropOverlimit'],
					'newFlowCount': item['stats']['newFlowCount'],
					'ecnMark': item['stats']['ecnMark'],
					'newFlowsLen': item['stats']['newFlowsLen'],
					'oldFlowsLen': item['stats']['oldFlowsLen'],
					'percentageDropped': item['stats']['percentageDropped'],
				}
			}
			mergedStats.append(newStat)
	for item in updatedFlowStats:
		if item['identification']['srcOrDst'] == 'dst':
			ipv4 = item['identification']['ipv4']
			ipv6 = item['identification']['ipv6']
			newStat = {
				'dst': {
					'GigabytesSent': item['stats']['GigabytesSent'],
					'PacketsSent': item['stats']['PacketsSent'],
					'droppedPackets': item['stats']['droppedPackets'],
					'overlimitsPackets': item['stats']['overlimitsPackets'],
					'requeuedPackets': item['stats']['requeuedPackets'] ,
					'backlogBytes': item['stats']['backlogBytes'],
					'backlogPackets': item['stats']['backlogPackets'],
					'requeues': item['stats']['requeues'],
					'maxPacket': item['stats']['maxPacket'],
					'dropOverlimit': item['stats']['dropOverlimit'],
					'newFlowCount': item['stats']['newFlowCount'],
					'ecnMark': item['stats']['ecnMark'],
					'newFlowsLen': item['stats']['newFlowsLen'],
					'oldFlowsLen': item['stats']['oldFlowsLen'],
					'percentageDropped': item['stats']['percentageDropped']
					}
			}
			for item2 in mergedStats:
				if ipv4 in item2['identification']['ipv4']:
					item2 = item2.update(newStat)
				elif ipv6 in item2['identification']['ipv6']:
					item2 = item2.update(newStat)
	return mergedStats
			
if __name__ == '__main__':
	mergedStats = getStatistics()
	
	# Display table of Customer CPEs with most packets dropped
	x = PrettyTable()
	x.field_names = ["Device", "AP", "IPv4", "IPv6", "UL Dropped", "DL Dropped", "GB Down/Up"]
	sortableList = []
	pickTop = 30
	for stat in mergedStats:
		name = stat['identification']['name']
		AP = stat['identification']['AP']
		ipv4 = stat['identification']['ipv4']
		ipv6 = stat['identification']['ipv6']
		srcDropped = stat['src']['percentageDropped']
		dstDropped = stat['dst']['percentageDropped']
		GBuploadedString = stat['src']['GigabytesSent']
		GBdownloadedString = stat['dst']['GigabytesSent']
		GBstring = GBuploadedString + '/' + GBdownloadedString
		avgDropped = (srcDropped + dstDropped)/2
		sortableList.append((name, AP, ipv4, ipv6, srcDropped, dstDropped, avgDropped, GBstring))
	res = sorted(sortableList, key = itemgetter(4), reverse = True)[:pickTop]
	for stat in res:
		name, AP, ipv4, ipv6, srcDropped, dstDropped, avgDropped, GBstring = stat
		if not name:
			name = ipv4
		srcDroppedString =  "{0:.4%}".format(srcDropped)
		dstDroppedString =  "{0:.4%}".format(dstDropped)
		x.add_row([name, AP, ipv4, ipv6, srcDroppedString, dstDroppedString, GBstring])
	print(x)
