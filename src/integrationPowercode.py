from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import warnings
from liblqos_python import find_ipv6_using_mikrotik, powercode_api_key, powercode_api_url
from integrationCommon import isIpv4Permitted
import base64
from requests.auth import HTTPBasicAuth
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType
from urllib3.exceptions import InsecureRequestWarning

def getCustomerInfo():
	headers= {'Content-Type': 'application/x-www-form-urlencoded'}
	url = powercode_api_url() + ":444/api/preseem/index.php"
	data = {}
	data['apiKey'] = powercode_api_key()
	data['action'] = 'list_customers'
	
	r = requests.post(url, data=data, headers=headers, verify=False, timeout=10)
	return r.json()

def getListServices():
	headers= {'Content-Type': 'application/x-www-form-urlencoded'}
	url = powercode_api_url() + ":444/api/preseem/index.php"
	data = {}
	data['apiKey'] = powercode_api_key()
	data['action'] = 'list_services'
	
	r = requests.post(url, data=data, headers=headers, verify=False, timeout=10)
	servicesDict = {}
	for service in r.json():
		if service['rate_down'] and service['rate_up']:
			servicesDict[service['id']] = {}
			servicesDict[service['id']]['downloadMbps'] = int(round(int(service['rate_down']) / 1000))
			servicesDict[service['id']]['uploadMbps'] = int(round(int(service['rate_up']) / 1000))
	return servicesDict

def createShaper():
	net = NetworkGraph()
	requests.packages.urllib3.disable_warnings(category=InsecureRequestWarning)
	print("Fetching data from Powercode")
	
	customerInfo = getCustomerInfo()
	
	customerIDs = []
	for customer in customerInfo:
		if customer['id'] != '1':
			if customer['id'] != '':
				if customer['status'] == 'Active':
					customerIDint = int(customer['id'])
					if customerIDint != 0:
						if customerIDint != None:
							if customerIDint not in customerIDs:
								customerIDs.append(customerIDint)
	
	allServices = getListServices()
	
	acceptableEquipment = ['Customer Owned Equipment', 'Router', 'Customer Owned Equipment', 'Managed Routers', 'CPE']
	
	devicesByCustomerID = {}
	for customer in customerInfo:
		if customer['status'] == 'Active':
			chosenName = ''
			if customer['name'] != '':
				chosenName = customer['name']
			elif customer['company_name'] != '':
				chosenName = customer['company_name']
			else:
				chosenName = customer['id']
			for equipment in customer['equipment']:
				if equipment['type'] in acceptableEquipment:
					if equipment['service_id'] in allServices:
						device = {}
						device['id'] = "c_" + customer['id'] + "_s_" + "_d_" + equipment['id']
						device['name'] = equipment['name']
						device['ipv4'] = equipment['ip_address']
						device['mac'] = equipment['mac_address']
						if customer['id'] not in devicesByCustomerID:
							devicesByCustomerID[customer['id']] = {}
							devicesByCustomerID[customer['id']]['name'] = chosenName
						devicesByCustomerID[customer['id']]['downloadMbps'] = allServices[equipment['service_id']]['downloadMbps']
						devicesByCustomerID[customer['id']]['uploadMbps'] = allServices[equipment['service_id']]['uploadMbps']
						if 'devices' not in devicesByCustomerID[customer['id']]:
							devicesByCustomerID[customer['id']]['devices'] = []
						devicesByCustomerID[customer['id']]['devices'].append(device)
	
	for customerID in devicesByCustomerID:
		customer = NetworkNode(
				type=NodeType.client,
				id=customerID,
				displayName=devicesByCustomerID[customerID]['name'],
				address='',
				customerName=devicesByCustomerID[customerID]['name'],
				download=devicesByCustomerID[customerID]['downloadMbps'],
				upload=devicesByCustomerID[customerID]['uploadMbps'],
			)
		net.addRawNode(customer)
		for device in devicesByCustomerID[customerID]['devices']:
			newDevice = NetworkNode(
				id=device['id'],
				displayName=device["name"],
				type=NodeType.device,
				parentId=customerID,
				mac=device["mac"],
				ipv4=[device['ipv4']],
				ipv6=[]
			)
			net.addRawNode(newDevice)
	print("Imported " + str(len(devicesByCustomerID)) + " customers")
	net.prepareTree()
	net.plotNetworkGraph(False)
	if net.doesNetworkJsonExist():
		print("network.json already exists. Leaving in-place.")
	else:
		net.createNetworkJson()
	net.createShapedDevices()

def importFromPowercode():
	#createNetworkJSON()
	createShaper()

if __name__ == '__main__':
	importFromPowercode()
