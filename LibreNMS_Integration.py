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
#                          v.0.65-alpha
#
import requests
from ispConfig import orgLibreNMSxAuthToken, libreNMSBaseURL, libreNMSDeviceGroups

def pullLibreNMSDevices():
	libreNMSDevicesToImport = []
	url = libreNMSBaseURL + "/api/v0/devicegroups/"
	headers = {'accept':'application/json', 'x-auth-token': orgLibreNMSxAuthToken}
	r = requests.get(url, headers=headers)
	allDevicesJSON = r.json()
	#print(jsonData)
	for group in allDevicesJSON['groups']:
		groupName = group['name']
		if groupName in libreNMSDeviceGroups:
			if libreNMSDeviceGroups[groupName]['downloadMbps']:
				url = libreNMSBaseURL + "/api/v0/devicegroups/" + groupName
				headers = {'accept':'application/json', 'x-auth-token': orgLibreNMSxAuthToken}
				r = requests.get(url, headers=headers)
				group = r.json()['devices']
				for device in group:
					deviceID = device['device_id']
					ipAddr, hostname = getLibreNMSDeviceInfo(deviceID)
					thisShapedDevice = {
						"identification": {
						  "name": ipAddr,
						  "hostname": hostname,
						  "ipAddr": ipAddr,
						  "mac": None,
						  "model": None,
						  "modelName": None,
						  "unmsSiteID": None,
						  "libreNMSSiteID": None
						},
						"qos": {
						  "downloadMbps": libreNMSDeviceGroups[groupName]['downloadMbps'],
						  "uploadMbps": libreNMSDeviceGroups[groupName]['uploadMbps'],
						  "accessPoint": None
						},
					}
					print("Imported device from LibreNMS: " + hostname)
					libreNMSDevicesToImport.append(thisShapedDevice)
	return libreNMSDevicesToImport

def getLibreNMSDeviceInfo(deviceID):
	#Get IP
	url = libreNMSBaseURL + "/api/v0/devices/" + str(deviceID) + "/ip"
	headers = {'accept':'application/json', 'x-auth-token': orgLibreNMSxAuthToken}
	r = requests.get(url, headers=headers)
	ipAddr = ''
	for address in r.json()['addresses']:
		thisIP = address['ipv4_address']
		#Ignore internal CPE router IPs. This assumes your CPE router internal network IPs are 192.168.X.X
		if '192.168' not in ipAddr:
			ipAddr = thisIP
	if '/' in ipAddr:
		ipAddr = ipAddr.split('/')[0]	
	#Get hostname
	url = libreNMSBaseURL + "/api/v0/devices/" + str(deviceID)
	headers = {'accept':'application/json', 'x-auth-token': orgLibreNMSxAuthToken}
	r = requests.get(url, headers=headers)
	hostname = r.json()['devices'][0]['hostname']
	return ((ipAddr, hostname))
