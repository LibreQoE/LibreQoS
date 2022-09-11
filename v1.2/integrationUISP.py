import requests
import os
import csv
import ipaddress
from ispConfig import UISPbaseURL, uispAuthToken, shapeRouterOrStation, allowedSubnets, ignoreSubnets, excludeSites, findIPv6usingMikrotik, bandwidthOverheadFactor, exceptionCPEs
import shutil
import json
if findIPv6usingMikrotik == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  

knownRouterModels = ['ACB-AC', 'ACB-ISP']
knownAPmodels = ['LTU-Rocket', 'RP-5AC', 'RP-5AC-Gen2', 'LAP-GPS', 'Wave-AP']

def isInAllowedSubnets(inputIP):
	isAllowed = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in allowedSubnets:
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isAllowed = True
	return isAllowed
	
def createTree(sites,accessPoints,bandwidthDL,bandwidthUL,siteParentDict,siteIDtoName,sitesWithParents,currentNode):
	currentNodeName = list(currentNode.items())[0][0]
	childrenList = []
	for site in sites:
		try:
			thisOnesParent = siteIDtoName[site['identification']['parent']['id']]
			if thisOnesParent == currentNodeName:
				childrenList.append(site['id'])
		except:
			thisOnesParent = None
	aps = []
	for ap in accessPoints:
		thisOnesParent = ap['device']['site']['name']
		if thisOnesParent == currentNodeName:
			if ap['device']['model'] in knownAPmodels:
				aps.append(ap['device']['name'])
	apDict = {}
	for ap in aps:
		maxDL = min(bandwidthDL[ap],bandwidthDL[currentNodeName])
		maxUL = min(bandwidthUL[ap],bandwidthUL[currentNodeName])
		apStruct = 	{
					ap : 
						{
							"downloadBandwidthMbps": maxDL,
							"uploadBandwidthMbps": maxUL,
						}
				}
		apDictNew = apDict | apStruct
		apDict = apDictNew
	if bool(apDict):
		currentNode[currentNodeName]['children'] = apDict
	counter = 0
	tempChildren = {}
	for child in childrenList:
		name = siteIDtoName[child]
		maxDL = min(bandwidthDL[name],bandwidthDL[currentNodeName])
		maxUL = min(bandwidthUL[name],bandwidthUL[currentNodeName])
		childStruct = 	{
							name : 
								{
									"downloadBandwidthMbps": maxDL,
									"uploadBandwidthMbps": maxUL,
								}
						}
		childStruct = createTree(sites,accessPoints,bandwidthDL,bandwidthUL,siteParentDict,siteIDtoName,sitesWithParents,childStruct)
		tempChildren = tempChildren | childStruct
		counter += 1
	if tempChildren != {}:
		if 'children' in currentNode[currentNodeName]:
			currentNode[currentNodeName]['children'] = currentNode[currentNodeName]['children'] | tempChildren
		else:
			currentNode[currentNodeName]['children'] = tempChildren
	return currentNode

def createNetworkJSON():
	print("Creating network.json")
	bandwidthDL = {}
	bandwidthUL = {}
	url = UISPbaseURL + "/nms/api/v2.1/sites?type=site"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	sites = r.json()
	url = UISPbaseURL + "/nms/api/v2.1/devices/aps/profiles"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	apProfiles = r.json()
	listOfTopLevelParentNodes = []	
	if os.path.isfile("integrationUISPbandwidths.csv"):
		with open('integrationUISPbandwidths.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				name, download, upload = row
				download = int(download)
				upload = int(upload)
				listOfTopLevelParentNodes.append(name)
				bandwidthDL[name] = download
				bandwidthUL[name] = upload
	for ap in apProfiles:
		name = ap['device']['name']
		model = ap['device']['model']
		apID = ap['device']['id']
		if model in knownAPmodels:
			url = UISPbaseURL + "/nms/api/v2.1/devices/airmaxes/" + apID + '?withStations=false'
			headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
			r = requests.get(url, headers=headers)
			thisAPairmax = r.json()	
			downloadCap = int(round(thisAPairmax['overview']['downlinkCapacity']/1000000))
			uploadCap = int(round(thisAPairmax['overview']['uplinkCapacity']/1000000))
			# If operator already included bandwidth definitions for this ParentNode, do not overwrite what they set
			if name not in listOfTopLevelParentNodes:
				print("Found " + name)
				listOfTopLevelParentNodes.append(name)
				bandwidthDL[name] = downloadCap
				bandwidthUL[name] = uploadCap
	for site in sites:
		name = site['identification']['name']
		if name not in excludeSites:
			# If operator already included bandwidth definitions for this ParentNode, do not overwrite what they set
			if name not in listOfTopLevelParentNodes:
				print("Found " + name)
				listOfTopLevelParentNodes.append(name)
				bandwidthDL[name] = 1000
				bandwidthUL[name] = 1000
	with open('integrationUISPbandwidths.csv', 'w') as csvfile:
		wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
		wr.writerow(['ParentNode', 'Download Mbps', 'Upload Mbps'])
		for device in listOfTopLevelParentNodes:
			entry = (device, bandwidthDL[device], bandwidthUL[device])
			wr.writerow(entry)
	url = UISPbaseURL + "/nms/api/v2.1/devices?role=ap"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	accessPoints = r.json()	
	siteIDtoName = {}
	siteParentDict = {}
	sitesWithParents = []
	topLevelSites = []
	for site in sites:
		siteIDtoName[site['id']] = site['identification']['name']
		try:
			siteParentDict[site['id']] = site['identification']['parent']['id']
			sitesWithParents.append(site['id'])
		except:
			siteParentDict[site['id']] = None
			if site['identification']['name'] not in excludeSites:
				topLevelSites.append(site['id'])
	tLname = siteIDtoName[topLevelSites.pop()]
	topLevelNode = {
					tLname : 
						{
							"downloadBandwidthMbps": bandwidthDL[tLname],
							"uploadBandwidthMbps": bandwidthUL[tLname],
						}
					}
	tree = createTree(sites,apProfiles, bandwidthDL, bandwidthUL, siteParentDict,siteIDtoName,sitesWithParents,topLevelNode)
	with open('network.json', 'w') as f:
		json.dump(tree, f, indent=4)

def createShaper():
	print("Creating ShapedDevices.csv")
	devicesToImport = []
	url = UISPbaseURL + "/nms/api/v2.1/sites?type=site"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	sites = r.json()
	siteIDtoName = {}
	for site in sites:
		siteIDtoName[site['id']] = site['identification']['name']
	url = UISPbaseURL + "/nms/api/v2.1/sites?type=client&ucrm=true&ucrmDetails=true"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	clientSites = r.json()
	url = UISPbaseURL + "/nms/api/v2.1/devices"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	allDevices = r.json()
	ipv4ToIPv6 = {}
	if findIPv6usingMikrotik:
		ipv4ToIPv6 = pullMikrotikIPv6()
	for uispClientSite in clientSites:
		#if (uispClientSite['identification']['status'] == 'active') and (uispClientSite['identification']['suspended'] == False):
		if (uispClientSite['identification']['suspended'] == False):
			foundCPEforThisClientSite = False
			if (uispClientSite['qos']['downloadSpeed']) and (uispClientSite['qos']['uploadSpeed']):
				downloadSpeedMbps = int(round(uispClientSite['qos']['downloadSpeed']/1000000))
				uploadSpeedMbps = int(round(uispClientSite['qos']['uploadSpeed']/1000000))
				address = uispClientSite['description']['address']
				uispClientSiteID = uispClientSite['id']
				
				UCRMclientID = uispClientSite['ucrm']['client']['id']
				siteName = uispClientSite['identification']['name']
				AP = 'none'
				thisSiteDevices = []
				#Look for station devices, use those to find AP name
				for device in allDevices:
					if device['identification']['site'] != None:
						if device['identification']['site']['id'] == uispClientSite['id']:
							deviceName = device['identification']['name']
							deviceRole = device['identification']['role']
							deviceModel = device['identification']['model']
							deviceModelName = device['identification']['modelName']
							if (deviceRole == 'station'):
								if device['attributes']['apDevice']:
									AP = device['attributes']['apDevice']['name']
				#Look for router devices, use those as shaped CPE
				for device in allDevices:
					if device['identification']['site'] != None:
						if device['identification']['site']['id'] == uispClientSite['id']:
							deviceModel = device['identification']['model']
							deviceName = device['identification']['name']
							deviceRole = device['identification']['role']
							if device['identification']['mac']:
								deviceMAC = device['identification']['mac'].upper()
							else:
								deviceMAC = ''
							if (deviceRole == 'router') or (deviceModel in knownRouterModels):
								ipv4 = device['ipAddress']
								if '/' in ipv4:
									ipv4 = ipv4.split("/")[0]
								ipv6 = ''
								if ipv4 in ipv4ToIPv6.keys():
									ipv6 = ipv4ToIPv6[ipv4]
								if isInAllowedSubnets(ipv4):
									deviceModel = device['identification']['model']
									deviceModelName = device['identification']['modelName']
									minSpeedDown = min(1,downloadSpeedMbps)
									minSpeedUp = min(1,uploadSpeedMbps)
									maxSpeedDown = round(bandwidthOverheadFactor*downloadSpeedMbps)
									maxSpeedUp = round(bandwidthOverheadFactor*uploadSpeedMbps)
									#Customers directly connected to Sites
									if deviceName in exceptionCPEs.keys():
										AP = exceptionCPEs[deviceName]
									if AP == 'none':
										try:
											AP = siteIDtoName[uispClientSite['identification']['parent']['id']]
										except:
											AP = 'none'
									devicesToImport.append((uispClientSiteID, address, '', deviceName, AP, deviceMAC, ipv4, ipv6, str(minSpeedDown), str(minSpeedUp), str(maxSpeedDown),str(maxSpeedUp),''))
									foundCPEforThisClientSite = True
			else:
				print("Failed to import devices from " + uispClientSite['description']['address'] + ". Missing QoS.")
			if foundCPEforThisClientSite != True:
				print("Failed to import devices for " + uispClientSite['description']['address'])
	
	with open('ShapedDevices.csv', 'w') as csvfile:
		wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
		wr.writerow(['Circuit ID', 'Circuit Name', 'Device ID', 'Device Name', 'Parent Node', 'MAC', 'IPv4', 'IPv6', 'Download Min', 'Upload Min', 'Download Max', 'Upload Max', 'Comment'])
		for device in devicesToImport:
			wr.writerow(device)

def importFromUISP():
	createNetworkJSON()
	createShaper()

if __name__ == '__main__':
	importFromUISP()
