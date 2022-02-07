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
from itertools import groupby

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
				if interface == interfaceA:
					device['packetLossDownload'] = packetLoss
					device['bytesSentDownload'] = bytesSent
					device['packetsDownload'] = packets
					device['dropsDownload'] = drops
					device['foundStats'] = True
				else:
					device['packetLossUpload'] = packetLoss
					device['bytesSentUpload'] = bytesSent
					device['packetsUpload'] = packets
					device['dropsUpload'] = drops
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
	x.field_names = ["Hostname", "ParentNode", "IPv4", "IPv6", "DL Dropped", "UL Dropped", "Avg Dropped", "GB Down", "GB Up"]
	sortableList = []
	pickTopCPEs = 5
	pickTopAPs = 10
	for device in devices:
		name = device['hostname']
		ParentNode = device['ParentNode']
		ipv4 = device['ipv4']
		ipv6 = device['ipv6']
		packetLossDownload = device['packetLossDownload']
		packetLossUpload = device['packetLossUpload']
		GBdownloadedString = str(round((device['bytesSentDownload']/1000000000),3))
		GBuploadedString = str(round((device['bytesSentUpload']/1000000000),3))
		GBstring = GBdownloadedString + '/' + GBuploadedString
		avgDropped = round((packetLossDownload + packetLossUpload)/2,3)
		sortableList.append((name, ParentNode, ipv4, ipv6, packetLossDownload, packetLossUpload, avgDropped, GBdownloadedString, GBuploadedString))
	res = sorted(sortableList, key = itemgetter(4), reverse = True)[:pickTopCPEs]
	for stat in res:
		name, AP, ipv4, ipv6, packetLossDownload, packetLossUpload, avgDropped, GBdownloadedString, GBuploadedString = stat
		if not name:
			name = ipv4
		downloadDroppedString =  "{0:.3%}".format(packetLossDownload)
		uploadDroppedString =  "{0:.3%}".format(packetLossUpload)
		avgDroppedString = "{0:.3%}".format(avgDropped)
		x.add_row([name, AP, ipv4, ipv6, downloadDroppedString, uploadDroppedString, avgDroppedString, GBdownloadedString, GBuploadedString])
	print(x)
	
	listOfParentNodes = []
	listOfParentNodesWithStats = []
	for device in devices:
		if device['ParentNode'] not in listOfParentNodes:
			listOfParentNodes.append(device['ParentNode'])
	for parentNode in listOfParentNodes:
		bytesSentDownloadDropped = 0
		bytesSentUploadDropped = 0
		bytesSentDownload = 0
		bytesSentUpload = 0
		packetsDownload = 0
		packetsUpload = 0
		packetsDownloadDropped = 0
		packetsUploadDropped = 0
		counter = 0
		for device in devices:
			if device['ParentNode'] == parentNode:
				bytesSentDownload += device['bytesSentDownload']
				bytesSentUpload += device['bytesSentUpload']
				packetsDownload += device['packetsDownload']
				packetsUpload += device['packetsUpload']
				packetsDownloadDropped += device['dropsDownload']
				packetsUploadDropped += device['dropsUpload']
				counter += 1
		if bytesSentDownload > 0:
			packetLossDownload = round(packetsDownloadDropped/packetsDownload,5)
		else:
			packetLossDownload = 0
		if bytesSentUpload > 0:
			packetLossUpload = round(packetsUploadDropped/packetsUpload,5)
		else:
			packetLossUpload = 0
		GBdownload = round((bytesSentDownload/1000000000),3)
		GBupload = round((bytesSentUpload/1000000000),3)
		packetLossAvg = (packetLossDownload+packetLossUpload)/2
		listOfParentNodesWithStats.append((parentNode,packetLossDownload,packetLossUpload,packetLossAvg, GBdownload,GBupload))
	res = sorted(listOfParentNodesWithStats, key = itemgetter(3), reverse = True)[:pickTopAPs]

	x = PrettyTable()
	x.field_names = ["ParentNode", "Download Dropped", "Upload Dropped", "Avg Dropped", "GB Down", "GB Up"]
	for stat in res:
		parentNode,packetLossDownload,packetLossUpload,packetLossAvg, GBdownload,GBupload = stat
		packetLossDownloadString =  "{0:.3%}".format(packetLossDownload)
		packetLossUploadString =  "{0:.3%}".format(packetLossUpload)
		avgLossString = "{0:.3%}".format(packetLossAvg)
		x.add_row([parentNode,packetLossDownloadString,packetLossUploadString,avgLossString, GBdownload,GBupload])
	print(x)
