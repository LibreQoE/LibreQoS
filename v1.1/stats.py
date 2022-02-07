import os
import subprocess
from subprocess import PIPE
import io
import decimal
import json
from operator import itemgetter 
from prettytable import PrettyTable
from ispConfig import fqOrCAKE, interfaceA, interfaceB
import decimal

def getStatistics():
	with open('qdiscs.json', 'r') as infile:
		devices = json.load(infile)
	interfaceList = [interfaceA, interfaceB]

	for interface in interfaceList:
		for device in devices:
			try:
				command = 'tc -j -s qdisc show dev ' + interface + ' parent ' + device['qdisc']
				commands = command.split(' ')
				tcShowResults = subprocess.run(commands, stdout=subprocess.PIPE).stdout.decode('utf-8')
				jsonVersion = json.loads(tcShowResults)
				drops = int(jsonVersion[0]['drops'])
				packets = int(jsonVersion[0]['packets'])
				bytesSent = int(jsonVersion[0]['bytes'])
				packetLoss = round((drops/packets),6)
				if interface == interfaceB:
					device['packetLossDownload'] = packetLoss
					device['bytesSentDownload'] = bytesSent
					device['foundStats'] = True
				else:
					device['packetLossUpload'] = packetLoss
					device['bytesSentUpload'] = bytesSent
					device['foundStats'] = True
			except:
				print("Failed to retrieve stats for device " + device['hostname'])
				device['foundStats'] = False
	devicesWhereWeFoundStats = []
	for device in devices:
		if device['foundStats'] == True:
			devicesWhereWeFoundStats.append(device)
	return devicesWhereWeFoundStats
			
if __name__ == '__main__':
	devices = getStatistics()
	
	# Display table of Customer CPEs with most packets dropped
	x = PrettyTable()
	x.field_names = ["Hostname", "ParentNode", "IPv4", "IPv6", "DL Dropped", "UL Dropped", "GB Down/Up"]
	sortableList = []
	pickTop = 30
	for device in devices:
		name = device['hostname']
		AP = device['ParentNode']
		ipv4 = device['ipv4']
		ipv6 = device['ipv6']
		packetLossDownload = device['packetLossDownload']
		packetLossUpload = device['packetLossUpload']
		GBdownloadedString = str(round((device['bytesSentDownload']/1000000000),3))
		GBuploadedString = str(round((device['bytesSentUpload']/1000000000),3))
		GBstring = GBuploadedString + '/' + GBdownloadedString
		avgDropped = (packetLossDownload + packetLossUpload)/2
		sortableList.append((name, AP, ipv4, ipv6, packetLossDownload, packetLossUpload, avgDropped, GBstring))
	res = sorted(sortableList, key = itemgetter(4), reverse = True)[:pickTop]
	for stat in res:
		name, AP, ipv4, ipv6, packetLossDownload, packetLossUpload, avgDropped, GBstring = stat
		if not name:
			name = ipv4
		downloadDroppedString =  "{0:.3%}".format(packetLossDownload)
		uploadDroppedString =  "{0:.3%}".format(packetLossUpload)
		x.add_row([name, AP, ipv4, ipv6, downloadDroppedString, uploadDroppedString, GBstring])
	print(x)
