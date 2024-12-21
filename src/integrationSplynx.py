
from pythonCheck import checkPythonVersion
checkPythonVersion()

import requests
import warnings
import os
import csv
from liblqos_python import exclude_sites, find_ipv6_using_mikrotik, bandwidth_overhead_factor, splynx_api_key, \
	splynx_api_secret, splynx_api_url, overwrite_network_json_always

from integrationCommon import isIpv4Permitted
import base64
from requests.auth import HTTPBasicAuth
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType
import os
import csv

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

def getMonitoring(headers):
	return spylnxRequest("admin/networking/monitoring", headers)

def createShaper():
	"""
	Main function to fetch data from Splynx, build the network graph, and shape devices.
	"""
	net = NetworkGraph()

	print("Fetching data from Spylnx")
	headers = buildHeaders()
	print("Fetching tariffs from Spylnx")
	tariff, downloadForTariffID, uploadForTariffID = getTariffs(headers)
	print("Fetching all customers from Spylnx")
	customers = getCustomers(headers)
	print("Fetching online customers from Spylnx")
	customersOnline = getCustomersOnline(headers)
	#ipForRouter, nameForRouterID, routerIdList = getRouters(headers)
	#sectorForRouter = getSectors(headers)
	print("Fetching services from Spylnx")
	allServices = getAllServices(headers)
	print("Fetching hardware monitoring from Spylnx")
	monitoring = getMonitoring(headers)
	#ipv4ByCustomerID, ipv6ByCustomerID = getAllIPs(headers)
	siteBandwidth = buildSiteBandwidths()
	print("Successfully fetched data from Spylnx")
	
	allParentNodes = []
	custIDtoParentNode = {}
	parentNodeIDCounter = 100000
	matched_via_primary_method = 0
	matched_via_alternate_method = 0
	matched_with_parent_node = 0
	
	print("Matching customer services to IPs")
	# First priority - see if clients are associated with a Network Site via the access_device parameter
	hardware_name = {}
	access_device_name = {}
	hardware_parent = {}
	for monitored_device in monitoring:
		hardware_name[monitored_device['id']] = monitored_device['title']
		if 'access_device' in monitored_device:
			if monitored_device['access_device'] == '1':
				access_device_name[monitored_device['id']] = monitored_device['title']
				if 'parent_id' in monitored_device:
					hardware_parent[monitored_device['id']] = monitored_device['parent_id']
	# If site/node has parent, include that as "parentName_nodeName"
	hardware_name_extended = {}
	for monitored_device in monitoring:
		hardware_name[monitored_device['id']] = monitored_device['title']
		if 'parent_id' in monitored_device:
			if monitored_device['id'] in hardware_parent:
				if hardware_parent[monitored_device['id']] in hardware_name:
					hardware_name_extended[monitored_device['id']] = hardware_name[hardware_parent[monitored_device['id']]] + "_" + monitored_device['title'] 
		if monitored_device['id'] not in hardware_name_extended:
			hardware_name_extended[monitored_device['id']] = monitored_device['title'] 
	for device_num in hardware_name:
		# Find parent name of hardware
		parent_name = ''
		parent_id = None	
		if device_num in hardware_parent.keys():
			parent_id = hardware_parent[device_num]
			parent_name = hardware_name_extended[parent_id]
		download = 10000
		upload = 10000
		nodeName = hardware_name_extended[device_num]
		if nodeName in siteBandwidth:
			download = siteBandwidth[nodeName]["download"]
			upload = siteBandwidth[nodeName]["upload"]
		node = NetworkNode(id=device_num, displayName=nodeName, type=NodeType.site,
						   parentId=parent_id, download=download, upload=upload, address=None)
		net.addRawNode(node)
	cust_id_to_name ={}
	for customer in customers:
		cust_id_to_name[customer['id']] = customer['name']
	service_ids_handled = []
	for serviceItem in allServices:
		address = ''
		parent_node_id = None
		if 'access_device' in serviceItem:
			if serviceItem['access_device'] != 0:
				if serviceItem['access_device'] in hardware_name:
					parent_node_id = serviceItem['access_device']
		#if 'geo' in serviceItem:
		#	if 'address' in serviceItem['geo']:
		#		address = serviceItem['geo']['address']
		if (serviceItem['ipv4'] != '') or (serviceItem['ipv6'] != ''):
			customer = NetworkNode(
				type=NodeType.client,
				id=parentNodeIDCounter,
				parentId=parent_node_id,
				displayName=cust_id_to_name[serviceItem['customer_id']],
				address=cust_id_to_name[serviceItem['customer_id']],
				customerName=cust_id_to_name[serviceItem['customer_id']],
				download=downloadForTariffID[serviceItem['tariff_id']],
				upload=uploadForTariffID[serviceItem['tariff_id']]
			)
			net.addRawNode(customer)
			
			device = NetworkNode(
				id=100000 + parentNodeIDCounter,
				displayName=cust_id_to_name[serviceItem['customer_id']],
				type=NodeType.device,
				parentId=parentNodeIDCounter,
				mac=serviceItem['mac'],
				ipv4=[serviceItem['ipv4']],
				ipv6=[serviceItem['ipv6']]
			)
			net.addRawNode(device)
			parentNodeIDCounter = parentNodeIDCounter + 1
			if serviceItem['id'] not in service_ids_handled:
				service_ids_handled.append(serviceItem['id'])
			matched_via_primary_method += 1
			if parent_node_id != None:
				matched_with_parent_node += 1

	# For any services not correctly handled the way we just tried, try an alternative way
	previously_unhandled_services = {}
	for serviceItem in allServices:
		if serviceItem['id'] not in service_ids_handled:
			#if serviceItem['status'] == 'active':
			if serviceItem["id"] not in previously_unhandled_services:
				previously_unhandled_services[serviceItem["id"]] = []
			temp = previously_unhandled_services[serviceItem["id"]]
			temp.append(serviceItem)
			previously_unhandled_services[serviceItem["id"]] = temp
	customerIDtoCustomerName = {}
	for customer in customers:
		customerIDtoCustomerName[customer['id']] = customer['name']
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
	customer_name_for_id = {}
	for customerJson in customers:
		customer_name_for_id[customerJson['id']] = customerJson["name"]
	customer_name_for_service = {}
	for customerJson in customersOnline:
		if customerJson['id'] in customer_name_for_id:
			customer_name_for_service[customerJson['service_id']] = customer_name_for_id[customerJson['id']]
	for service in allServices:
		if service["id"] in previously_unhandled_services:
			if service["id"] not in service_ids_handled:
				if service["id"] in ipv4sForService:
					ipv4 = ipv4sForService[service["id"]]
				else:
					ipv4 = []
				if service["id"] in ipv6sForService:
					ipv6 = ipv6sForService[service["id"]]
				else:
					ipv6 = []
				customer_name = ''
				if service["id"] in customer_name_for_service:
					customer_name = customer_name_for_service[service["id"]]
				customer = NetworkNode(
					type=NodeType.client,
					id=service["id"],
					parentId=None,
					displayName=customer_name,
					address=customer_name,
					customerName=customer_name,
					download=downloadForTariffID[service['tariff_id']],
					upload=uploadForTariffID[service['tariff_id']]
				)
				net.addRawNode(customer)
				device = NetworkNode(
					id=service["id"],
					displayName=service["id"],
					type=NodeType.device,
					parentId=service["id"],
					mac=service["mac"],
					ipv4=ipv4,
					ipv6=ipv6
				)
				net.addRawNode(device)
				matched_via_alternate_method += 1
				if service["id"] not in service_ids_handled:
					service_ids_handled.append(service["id"])
	print("Matched " + "{:.0%}".format(len(service_ids_handled)/len(allServices)) + " of known services in Splynx.")
	print("Matched " + "{:.0%}".format(matched_via_primary_method/len(service_ids_handled)) + " services via primary method.")
	print("Matched " + "{:.0%}".format(matched_via_alternate_method/len(service_ids_handled)) + " services via alternate method.")
	print("Matched " + "{:.0%}".format(matched_with_parent_node/len(service_ids_handled)) + " of services found with their corresponding parent node.")
	
	net.prepareTree()
	net.plotNetworkGraph(False)
	
	if net.doesNetworkJsonExist():
		if overwrite_network_json_always:
			net.createNetworkJson()
		else:
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
