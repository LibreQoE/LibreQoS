from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import warnings
import os
import csv
from liblqos_python import exclude_sites, find_ipv6_using_mikrotik, bandwidth_overhead_factor, splynx_api_key, \
	splynx_api_secret, splynx_api_url
from integrationCommon import isIpv4Permitted
import base64
from requests.auth import HTTPBasicAuth
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType

def buildHeaders():
	"""
	Build authorization headers for Splynx API requests using API key and secret.
	"""
	credentials = splynx_api_key() + ':' + splynx_api_secret()
	credentials = base64.b64encode(credentials.encode()).decode()
	return {'Authorization': "Basic %s" % credentials}

def spylnxRequest(target, headers):
	"""
	Send a GET request to the Splynx API and return the JSON response.
	"""
	url = splynx_api_url() + "/api/2.0/" + target
	r = requests.get(url, headers=headers, timeout=120)
	return r.json()

def getTariffs(headers):
	"""
	Retrieve tariff data from Splynx API and calculate download/upload speeds for each tariff.
	"""
	data = spylnxRequest("admin/tariffs/internet", headers)
	downloadForTariffID = {}
	uploadForTariffID = {}
	try:
		for tariff in data:
			tariffID = tariff['id']
			speed_download = round((int(tariff['speed_download']) / 1024))
			speed_upload = round((int(tariff['speed_upload']) / 1024))
			downloadForTariffID[tariffID] = speed_download
			uploadForTariffID[tariffID] = speed_upload
	except:
		print("Error, bad data returned from Splynx:")
		print(data)
	return (data, downloadForTariffID, uploadForTariffID)

def buildSiteBandwidths():
	"""
	Build a dictionary of site bandwidths by reading data from a CSV file.
	"""
	siteBandwidth = {}
	if os.path.isfile("integrationSplynxBandwidths.csv"):
		with open('integrationSplynxBandwidths.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				name, download, upload = row
				download = int(float(download))
				upload = int(float(upload))
				siteBandwidth[name] = {"download": download, "upload": upload}
	return siteBandwidth

def getCustomers(headers):
	"""
	Retrieve all customer data from Splynx API.
	"""
	return spylnxRequest("admin/customers/customer", headers)

def getCustomersOnline(headers):
	"""
	Retrieve data of currently online customers from Splynx API.
	"""
	return spylnxRequest("admin/customers/customers-online", headers)

def getRouters(headers):
	"""
	Retrieve router data from Splynx API and build dictionaries for router IPs and names.
	"""
	data = spylnxRequest("admin/networking/routers", headers)
	routerIdList = []
	ipForRouter = {}
	nameForRouterID = {}
	for router in data:
		routerID = router['id']
		if router['id'] not in routerIdList:
			routerIdList.append(router['id'])
		ipForRouter[routerID] = router['ip']
		nameForRouterID[routerID] = router['title']
	print("Router IPs found: " + str(len(ipForRouter)))
	return (ipForRouter, nameForRouterID, routerIdList)

def getSectors(headers):
	"""
	Retrieve sector data from Splynx API and build a dictionary mapping routers to their sectors.
	"""
	data = spylnxRequest("admin/networking/routers-sectors", headers)
	sectorForRouter = {}
	for sector in data:
		routerID = sector['router_id']
		if routerID not in sectorForRouter:
			newList = []
			newList.append(sector)
			sectorForRouter[routerID] = newList
		else:
			newList = sectorForRouter[routerID]
			newList.append(sector)
			sectorForRouter[routerID] = newList
			
	print("Router Sectors found: " + str(len(sectorForRouter)))
	return sectorForRouter

def combineAddress(json):
	"""
	Combine address fields into a single string. If address fields are empty, use ID and name.
	"""
	if 'street_1' in json:
		if json["street_1"] == "" and json["city"] == "" and json["zip_code"] == "":
			return str(json["name"])
		else:
			return json["street_1"] + " " + json["city"] + " " + json["zip_code"]
	else:
		return str(json["name"])

def getAllServices(headers):
	"""
	Retrieve all active internet services from Splynx API.
	"""
	return spylnxRequest("admin/customers/customer/0/internet-services?main_attributes%5Bstatus%5D=active", headers)

def getAllIPs(headers):
	"""
	Retrieve all used IPv4 and IPv6 addresses from Splynx API and map them to customer IDs.
	"""
	ipv4ByCustomerID = {}
	ipv6ByCustomerID = {}
	allIPv4 = spylnxRequest("admin/networking/ipv4-ip?main_attributes%5Bis_used%5D=1", headers)
	allIPv6 = spylnxRequest("admin/networking/ipv6-ip", headers)
	for ipv4 in allIPv4:
		if ipv4['customer_id'] not in ipv4ByCustomerID:
			ipv4ByCustomerID[ipv4['customer_id']] = []
		temp = ipv4ByCustomerID[ipv4['customer_id']]
		temp.append(ipv4['ip'])
		ipv4ByCustomerID[ipv4['customer_id']] = temp
	for ipv6 in allIPv6:
		if ipv6['is_used'] == 1:
			if ipv6['customer_id'] not in ipv6ByCustomerID:
				ipv6ByCustomerID[ipv6['customer_id']] = []
			temp = ipv6ByCustomerID[ipv6['customer_id']]
			temp.append(ipv6['ip'])
			ipv6ByCustomerID[ipv6['customer_id']] = temp
	return (ipv4ByCustomerID, ipv6ByCustomerID)

def createShaper():
	"""
	Main function to fetch data from Splynx, build the network graph, and shape devices.
	"""
	net = NetworkGraph()

	print("Fetching data from Spylnx")
	headers = buildHeaders()
	tariff, downloadForTariffID, uploadForTariffID = getTariffs(headers)
	customers = getCustomers(headers)
	customersOnline = getCustomersOnline(headers)
	#ipForRouter, nameForRouterID, routerIdList = getRouters(headers)
	sectorForRouter = getSectors(headers)
	allServices = getAllServices(headers)
	#ipv4ByCustomerID, ipv6ByCustomerID = getAllIPs(headers)
	siteBandwidth = buildSiteBandwidths()
	
	allParentNodes = []
	custIDtoParentNode = {}
	parentNodeIDCounter = 30000
	
	# Create nodes for sites and assign bandwidth
	for customer in customersOnline:
		download = 10000
		upload = 10000
		nodeName = str(customer['nas_id']) + "_" + str(customer['call_to']).replace('.','_') + "_" + str(customer['port'])
		
		if nodeName not in allParentNodes:
			if nodeName in siteBandwidth:
				download = siteBandwidth[nodeName]["download"]
				upload = siteBandwidth[nodeName]["upload"]
			
			node = NetworkNode(id=parentNodeIDCounter, displayName=nodeName, type=NodeType.site,
							   parentId=None, download=download, upload=upload, address=None)
			net.addRawNode(node)
			
			pnEntry = {}
			pnEntry['name'] = nodeName
			pnEntry['id'] = parentNodeIDCounter
			custIDtoParentNode[customer['customer_id']] = pnEntry
			
			parentNodeIDCounter += 1

	allServicesDict = {}
	for serviceItem in allServices:
		if serviceItem['status'] == 'active':
			if serviceItem["customer_id"] not in allServicesDict:
				allServicesDict[serviceItem["customer_id"]] = []
			temp = allServicesDict[serviceItem["customer_id"]]
			temp.append(serviceItem)
			allServicesDict[serviceItem["customer_id"]] = temp
	customerIDtoCustomerName = {}
	for customer in customers:
		customerIDtoCustomerName[customer['id']] = customer['name']
	#print(customerIDtoCustomerName)
	# Create nodes for customers and their devices
	alreadyObservedIPv4s = []
	alreadyObservedCombinedIDs = []
	ipv4sForService= {}
	ipv6sForService= {}
	for customerJson in customersOnline:
		ipv4 = customerJson['ipv4']
		ipv6 = customerJson['ipv6']
		if customerJson['service_id'] in ipv4sForService:
			temp = ipv4sForService[customerJson['service_id']]
		else:
			temp = []
		if ipv4 not in temp:
			temp.append(ipv4)
		ipv4sForService[customerJson['service_id']] = temp
		
		if customerJson['service_id'] in ipv6sForService:
			temp = ipv6sForService[customerJson['service_id']]
		else:
			temp = []
		if ipv6 not in temp:
			temp.append(ipv6)
		ipv6sForService[customerJson['service_id']] = temp
				
	for customerJson in customers:
		#if customerJson['status'] == 'active':
		if customerJson['id'] in allServicesDict:
			servicesForCustomer = allServicesDict[customerJson['id']]
			for service in servicesForCustomer:
				if service["id"] in ipv4sForService:
					ipv4 = ipv4sForService[service["id"]]
				else:
					ipv4 = []
				if service["id"] in ipv6sForService:
					ipv6 = ipv6sForService[service["id"]]
				else:
					ipv6 = []
				
				combinedId = "c_" + str(customerJson["id"]) + "_s_" + str(service["id"])
				combinedId = combinedId.replace('.','_')
				tariff_id = service['tariff_id']
				
				parentID = None
				if customerJson['id'] in custIDtoParentNode:
					parentID = custIDtoParentNode[customerJson['id']]['id']
				
				customer = NetworkNode(
					type=NodeType.client,
					id=combinedId,
					parentId=parentID,
					displayName=customerJson["name"],
					address=combineAddress(customerJson),
					customerName=customerJson["name"],
					download=downloadForTariffID[tariff_id],
					upload=uploadForTariffID[tariff_id]
				)
				net.addRawNode(customer)
				
				device = NetworkNode(
					id=combinedId + "_d" + str(service["id"]),
					displayName=service["id"],
					type=NodeType.device,
					parentId=combinedId,
					mac=service["mac"],
					ipv4=ipv4,
					ipv6=ipv6
				)
				net.addRawNode(device)
	
	
	net.prepareTree()
	net.plotNetworkGraph(False)
	if net.doesNetworkJsonExist():
		print("network.json already exists. Leaving in-place.")
	else:
		net.createNetworkJson()
	net.createShapedDevices()

def importFromSplynx():
	"""
	Entry point for the script to initiate the Splynx data import and shaper creation process.
	"""
	createShaper()

if __name__ == '__main__':
	importFromSplynx()
