import requests
import csv
import ipaddress
from ispConfig import UISPbaseURL, uispAuthToken, shapeRouterOrStation, ignoreSubnets
import shutil

stationModels = ['LBE-5AC-Gen2', 'LBE-5AC-Gen2', 'LBE-5AC-LR', 'AF-LTU5', 'AFLTULR', 'AFLTUPro', 'LTU-LITE']
routerModels = ['ACB-AC', 'ACB-ISP']

def pullShapedDevices():
	devices = []
	uispSitesToImport = []
	url = UISPbaseURL + "/nms/api/v2.1/sites?type=client&ucrm=true&ucrmDetails=true"
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	jsonData = r.json()
	uispDevicesToImport = []
	for uispClientSite in jsonData:
		if (uispClientSite['identification']['status'] == 'active'):
			if (uispClientSite['qos']['downloadSpeed']) and (uispClientSite['qos']['uploadSpeed']):
				downloadSpeedMbps = int(round(uispClientSite['qos']['downloadSpeed']/1000000))
				uploadSpeedMbps = int(round(uispClientSite['qos']['uploadSpeed']/1000000))
				address = uispClientSite['description']['address']
				uispClientSiteID = uispClientSite['id']
				devicesInUISPsite = getUISPdevicesAtClientSite(uispClientSiteID)
				UCRMclientID = uispClientSite['ucrm']['client']['id']
				AP = 'none'
				thisSiteDevices = []
				#Look for station devices, use those to find AP name
				for device in devicesInUISPsite:
					deviceName = device['identification']['name']
					deviceRole = device['identification']['role']
					deviceModel = device['identification']['model']
					deviceModelName = device['identification']['modelName']
					if (deviceRole == 'station') or (deviceModel in stationModels):
						if device['attributes']['apDevice']:
							AP = device['attributes']['apDevice']['name']
				if shapeRouterOrStation == 'router':
					#Look for router devices, use those as shaped CPE
					for device in devicesInUISPsite:
						deviceName = device['identification']['name']
						deviceRole = device['identification']['role']
						deviceMAC = device['identification']['mac']
						deviceIPstring = device['ipAddress']
						if '/' in deviceIPstring:
							deviceIPstring = deviceIPstring.split("/")[0]
						deviceModel = device['identification']['model']
						deviceModelName = device['identification']['modelName']
						if (deviceRole == 'router') or (deviceModel in routerModels):
							print("Added " + ":\t" + deviceName)
							devices.append((UCRMclientID, AP,deviceMAC, deviceName, deviceIPstring,'', str(downloadSpeedMbps/4), str(uploadSpeedMbps/4), str(downloadSpeedMbps),str(uploadSpeedMbps)))
				elif shapeRouterOrStation == 'station':
					#Look for station devices, use those as shaped CPE
					for device in devicesInUISPsite:
						deviceName = device['identification']['name']
						deviceRole = device['identification']['role']
						deviceMAC = device['identification']['mac']
						deviceIPstring = device['ipAddress']
						if '/' in deviceIPstring:
							deviceIPstring = deviceIPstring.split("/")[0]
						deviceModel = device['identification']['model']
						deviceModelName = device['identification']['modelName']
						if (deviceRole == 'station') or (deviceModel in stationModels):
							print("Added " + ":\t" + deviceName)
							devices.append((UCRMclientID, AP,deviceMAC, deviceName, deviceIPstring,'', str(round(downloadSpeedMbps/4)), str(round(uploadSpeedMbps/4)), str(downloadSpeedMbps),str(uploadSpeedMbps)))
				uispSitesToImport.append(thisSiteDevices)
				print("Imported " + address)
			else:
				print("Failed to import devices from " + uispClientSite['description']['address'] + ". Missing QoS.")
	return devices

def getUISPdevicesAtClientSite(siteID):
	url = UISPbaseURL + "/nms/api/v2.1/devices?siteId=" + siteID
	headers = {'accept':'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers)
	return (r.json())

def updateFromUISP():
	# Copy file shaper to backup in case of power loss during write of new version
	shutil.copy('Shaper.csv', 'Shaper.csv.bak')
	
	devicesFromShaperCSV = []
	with open('Shaper.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			deviceID, ParentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
			ipv4 = ipv4.strip()
			ipv6 = ipv6.strip()
			ParentNode = ParentNode.strip()
			devicesFromShaperCSV.append((deviceID, ParentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax))
	
	#Make list of IPs, so that we can check if a device being imported is already entered in Shaper.csv
	devicesPulledFromUISP = pullShapedDevices()
	mergedDevicesList = devicesFromShaperCSV
	ipv4List = []
	ipv6List = []
	for device in devicesFromShaperCSV:
		deviceID, ParentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = device
		if (ipv4 != ''):
			ipv4List.append(ipv4)
		if (ipv6 != ''):
			ipv6List.append(ipv6)
	
	#For each device brought in from UISP, check if its in excluded subnets. If not, add it to Shaper.csv
	for device in devicesPulledFromUISP:
		deviceID, ParentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = device
		isThisIPexcludable = False
		for subnet in ignoreSubnets:
			if ipaddress.ip_address(ipv4) in ipaddress.ip_network(subnet):
				isThisIPexcludable = True
		if (isThisIPexcludable == False) and (ipv4 not in ipv4List):
			mergedDevicesList.append(device)
			
	with open('Shaper.csv', 'w') as csvfile:
		wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
		wr.writerow(['ID', 'AP', 'MAC', 'Hostname', 'IPv4', 'IPv6', 'Download Min', 'Upload Min', 'Download Max', 'Upload Max'])
		for device in mergedDevicesList:
			wr.writerow(device)

if __name__ == '__main__':
	updateFromUISP()
