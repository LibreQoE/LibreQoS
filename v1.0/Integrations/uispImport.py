import requests
import csv
from ispConfig.py import UISPbaseURL, uispAuthToken, shapeRouterOrStation


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
							devices.append((UCRMclientID, AP,deviceMAC, deviceName, deviceIPstring,'', str(downloadSpeedMbps/4), str(uploadSpeedMbps/4), str(downloadSpeedMbps),str(uploadSpeedMbps)))
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

if __name__ == '__main__':
	devicesList = pullShapedDevices()
	with open('Shaper.csv', 'w') as csvfile:
		wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
		wr.writerow(['ID', 'AP', 'MAC', 'Hostname', 'IPv4', 'IPv6', 'Download Min', 'Upload Min', 'Download Max', 'Upload Max'])
		for device in devicesList:
			wr.writerow(device)
