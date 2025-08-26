
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
			speed_download = float(tariff['speed_download']) / 1024
			speed_upload = float(tariff['speed_upload']) / 1024
			if ('burst_limit_fixed_down' in tariff) and ('burst_limit_fixed_up' in tariff):
				burstable_down = float(tariff['burst_limit_fixed_down']) / 1024
				burstable_up = float(tariff['burst_limit_fixed_up']) / 1024
				if burstable_down > speed_download:
					speed_download = burstable_down
				if burstable_up > speed_upload:
					speed_upload = burstable_up
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

def getNetworkSites(headers):
	"""
	Retrieve network sites data from Splynx API for better topology mapping.
	"""
	return spylnxRequest("admin/networking/network-sites", headers)

def findBestParentNode(serviceItem, hardware_name, ipForRouter, sectorForRouter, networkSites):
	"""
	Find the best parent node for a service using multiple methods.
	Returns tuple: (parent_node_id, assignment_method)
	"""
	parent_node_id = None
	assignment_method = 'none'
	
	# Method 1: Direct access_device assignment (highest priority)
	if 'access_device' in serviceItem and serviceItem['access_device'] != 0:
		if serviceItem['access_device'] in hardware_name:
			parent_node_id = serviceItem['access_device']
			assignment_method = 'access_device'
			return parent_node_id, assignment_method
	
	# Method 2: Router ID assignment
	if 'router_id' in serviceItem and serviceItem['router_id'] != 0:
		router_id = serviceItem['router_id']
		if router_id in hardware_name:
			parent_node_id = router_id
			assignment_method = 'router_id'
			return parent_node_id, assignment_method
		# Check if router has sectors
		if router_id in sectorForRouter:
			sectors = sectorForRouter[router_id]
			if sectors and len(sectors) > 0:
				# Use first sector as parent
				sector = sectors[0]
				if 'id' in sector and sector['id'] in hardware_name:
					parent_node_id = sector['id']
					assignment_method = 'sector_id'
					return parent_node_id, assignment_method
	
	# Method 3: Network Site assignment via location
	if networkSites and 'geo' in serviceItem:
		service_geo = serviceItem.get('geo', {})
		if 'lat' in service_geo and 'lng' in service_geo:
			service_lat = float(service_geo['lat'])
			service_lng = float(service_geo['lng'])
			
			# Find nearest network site
			min_distance = float('inf')
			nearest_site = None
			
			for site in networkSites:
				if 'geo' in site and 'lat' in site['geo'] and 'lng' in site['geo']:
					site_lat = float(site['geo']['lat'])
					site_lng = float(site['geo']['lng'])
					# Simple Euclidean distance (good enough for small areas)
					distance = ((service_lat - site_lat) ** 2 + (service_lng - site_lng) ** 2) ** 0.5
					if distance < min_distance:
						min_distance = distance
						nearest_site = site
			
			if nearest_site and 'id' in nearest_site:
				if nearest_site['id'] in hardware_name:
					parent_node_id = nearest_site['id']
					assignment_method = 'network_site'
					return parent_node_id, assignment_method
	
	return parent_node_id, assignment_method

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
	print("Fetching routers from Spylnx")
	ipForRouter, nameForRouterID, routerIdList = getRouters(headers)
	print("Fetching sectors from Spylnx")
	sectorForRouter = getSectors(headers)
	print("Fetching services from Spylnx")
	allServices = getAllServices(headers)
	print("Fetching hardware monitoring from Spylnx")
	monitoring = getMonitoring(headers)
	# Try to fetch network sites, but continue if it fails (not all Splynx installations have this)
	networkSites = []
	try:
		print("Fetching network sites from Spylnx")
		networkSites = getNetworkSites(headers)
		print(f"Found {len(networkSites)} network sites")
	except Exception as e:
		print(f"Warning: Could not fetch network sites (may not be available): {e}")
		networkSites = []
	#ipv4ByCustomerID, ipv6ByCustomerID = getAllIPs(headers)
	siteBandwidth = buildSiteBandwidths()
	print("Successfully fetched data from Spylnx")
	
	allParentNodes = []
	custIDtoParentNode = {}
	parentNodeIDCounter = 100000
	matched_via_primary_method = 0
	matched_via_alternate_method = 0
	matched_with_parent_node = 0
	# Track parent node assignment methods
	parent_assignment_methods = {
		'access_device': 0,
		'router_id': 0,
		'sector_id': 0,
		'network_site': 0,
		'geographic': 0,
		'none': 0
	}
	
	print("Matching customer services to IPs")
	# First priority - see if clients are associated with a Network Site via the access_device parameter
	hardware_name = {}
	access_device_name = {}
	hardware_parent = {}
	hardware_type = {}
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
		if 'type' in monitored_device:
			if monitored_device['type'] == 5:
				hardware_type[monitored_device['id']] = 'AP'
			else:
				hardware_type[monitored_device['id']] = 'Site'
	for device_num in hardware_name:
		parent_id = None	
		if device_num in hardware_parent.keys():
			if hardware_parent[device_num] != 0:
				parent_id = hardware_parent[device_num]
		download = 10000
		upload = 10000
		nodeName = hardware_name_extended[device_num]
		if nodeName in siteBandwidth:
			download = siteBandwidth[nodeName]["download"]
			upload = siteBandwidth[nodeName]["upload"]
		nodeType = hardware_type[device_num]
		if nodeType == 'AP':
			node = NetworkNode(id=device_num, displayName=nodeName, type=NodeType.ap,
				parentId=parent_id, download=download, upload=upload, address=None)
		else:
			node = NetworkNode(id=device_num, displayName=nodeName, type=NodeType.site,
				parentId=parent_id, download=download, upload=upload, address=None)
		net.addRawNode(node)
	cust_id_to_name ={}
	for customer in customers:
		cust_id_to_name[customer['id']] = customer['name']
	service_ids_handled = []
	allocated_ipv4s = {}
	allocated_ipv6s = {}
	# Track circuit IDs by customer+location to handle multiple locations per customer
	# Key format: "customer_id:parent_node_id" or "customer_id:service_address"
	circuit_id_by_customer_location = {}
	# Track circuit bandwidth to handle aggregation for multiple services at same location
	circuit_bandwidth = {}
	device_counter = 200000
	for serviceItem in allServices:
		if serviceItem['status'] == 'active':
			address = ''
			ipv4 = ''
			ipv6 = ''
			if serviceItem['ipv4'] != '':
				if serviceItem['ipv4'] not in allocated_ipv4s:
					ipv4 = serviceItem['ipv4']
					allocated_ipv4s[serviceItem['ipv4']] = True
				else:
					print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IP of " + serviceItem['ipv4'] + ". IP omitted.")
			if serviceItem['ipv6'] != '':
				if serviceItem['ipv6'] not in allocated_ipv6s:
					ipv6 = serviceItem['ipv6']
					allocated_ipv6s[serviceItem['ipv6']] = True
				else:
					print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IP of " + serviceItem['ipv6'] + ". IP omitted.")
			# Find best parent node using enhanced logic
			parent_node_id, assignment_method = findBestParentNode(
				serviceItem, hardware_name, ipForRouter, sectorForRouter, networkSites
			)
			parent_assignment_methods[assignment_method] += 1
			
			#if 'geo' in serviceItem:
			#	if 'address' in serviceItem['geo']:
			#		address = serviceItem['geo']['address']
			if (ipv4 != '') or (ipv6 != ''):
				customer_id = serviceItem['customer_id']
				customer_name = cust_id_to_name.get(customer_id, str(customer_id))
				
				# Get service bandwidth
				service_download = downloadForTariffID[serviceItem['tariff_id']]
				service_upload = uploadForTariffID[serviceItem['tariff_id']]
				
				# Create unique key for customer+location
				# Use parent node if available, otherwise use service address or ID
				location_key = str(parent_node_id) if parent_node_id else ""
				if not location_key and 'geo' in serviceItem:
					# Use geographic address if available
					if 'address' in serviceItem['geo']:
						location_key = serviceItem['geo']['address']
				if not location_key:
					# Fall back to service ID to ensure uniqueness
					location_key = f"service_{serviceItem['id']}"
				
				circuit_key = f"{customer_id}:{location_key}"
				
				# Check if we already have a circuit ID for this customer+location
				if circuit_key in circuit_id_by_customer_location:
					circuit_id = circuit_id_by_customer_location[circuit_key]
					# Circuit already exists, update bandwidth if this service has higher speeds
					if circuit_id in circuit_bandwidth:
						current_dl, current_ul = circuit_bandwidth[circuit_id]
						if service_download > current_dl or service_upload > current_ul:
							# Update to use the maximum bandwidth
							new_download = max(service_download, current_dl)
							new_upload = max(service_upload, current_ul)
							circuit_bandwidth[circuit_id] = (new_download, new_upload)
							# Update the existing circuit node with new bandwidth
							for node in net.nodes:
								if node.id == circuit_id and node.type == NodeType.client:
									node.download = new_download
									node.upload = new_upload
									break
				else:
					# New customer+location combination, create circuit
					circuit_id = parentNodeIDCounter
					circuit_id_by_customer_location[circuit_key] = circuit_id
					circuit_bandwidth[circuit_id] = (service_download, service_upload)
					
					# Create the customer circuit node
					customer = NetworkNode(
						type=NodeType.client,
						id=circuit_id,
						parentId=parent_node_id,
						displayName=customer_name,
						address=customer_name,
						customerName=customer_name,
						download=service_download,
						upload=service_upload
					)
					net.addRawNode(customer)
					parentNodeIDCounter = parentNodeIDCounter + 1
				
				# Always create a device for each service
				device = NetworkNode(
					id=device_counter,
					displayName=customer_name,
					type=NodeType.device,
					parentId=circuit_id,
					mac=serviceItem['mac'],
					ipv4=[ipv4] if ipv4 else [],
					ipv6=[ipv6] if ipv6 else []
				)
				net.addRawNode(device)
				device_counter = device_counter + 1
				
				if serviceItem['id'] not in service_ids_handled:
					service_ids_handled.append(serviceItem['id'])
				matched_via_primary_method += 1
				if parent_node_id != None:
					matched_with_parent_node += 1

	# Build IP mappings from customersOnline for supplementation
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
	
	# Intermediate step: Supplement primary method results with IPs from customersOnline
	matched_via_supplementation = 0
	for serviceItem in allServices:
		if serviceItem['id'] in service_ids_handled and serviceItem['status'] == 'active':
			# Check if we can supplement missing IPs for already handled services
			# Find the device node that was created for this service
			for node in net.nodes:
				if (node.type == NodeType.device and 
					node.displayName == cust_id_to_name.get(serviceItem['customer_id'], '')):
					needs_supplement = False
					supplemented_ipv4 = []
					supplemented_ipv6 = []
					
					# Check if IPv4 needs supplementation
					if (not node.ipv4 or node.ipv4 == ['']) and serviceItem['id'] in ipv4sForService:
						for ipv4 in ipv4sForService[serviceItem['id']]:
							if ipv4 and ipv4 not in allocated_ipv4s:
								supplemented_ipv4.append(ipv4)
								allocated_ipv4s[ipv4] = True
								needs_supplement = True
					
					# Check if IPv6 needs supplementation
					if (not node.ipv6 or node.ipv6 == ['']) and serviceItem['id'] in ipv6sForService:
						for ipv6 in ipv6sForService[serviceItem['id']]:
							if ipv6 and ipv6 not in allocated_ipv6s:
								supplemented_ipv6.append(ipv6)
								allocated_ipv6s[ipv6] = True
								needs_supplement = True
					
					if needs_supplement:
						if supplemented_ipv4:
							node.ipv4 = supplemented_ipv4
						if supplemented_ipv6:
							node.ipv6 = supplemented_ipv6
						matched_via_supplementation += 1
						print(f"Supplemented IPs for {cust_id_to_name.get(serviceItem['customer_id'], 'Unknown')} - IPv4: {supplemented_ipv4}, IPv6: {supplemented_ipv6}")
					break
	
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
				
				# Get customer info
				customer_id = service.get('customer_id', service["id"])
				customer_name = ''
				if service["id"] in customer_name_for_service:
					customer_name = customer_name_for_service[service["id"]]
				elif customer_id in customer_name_for_id:
					customer_name = customer_name_for_id[customer_id]
				
				# Get service bandwidth
				service_download = downloadForTariffID[service['tariff_id']]
				service_upload = uploadForTariffID[service['tariff_id']]
				
				# Create unique key for customer+location
				# For alternate method, we don't have parent nodes, so use service-specific key
				location_key = ""
				if 'geo' in service and 'address' in service['geo']:
					location_key = service['geo']['address']
				if not location_key:
					# Use service ID to ensure uniqueness for each service
					location_key = f"service_{service['id']}"
				
				circuit_key = f"{customer_id}:{location_key}"
				
				# Check if we already have a circuit ID for this customer+location
				if circuit_key in circuit_id_by_customer_location:
					circuit_id = circuit_id_by_customer_location[circuit_key]
					# Circuit already exists, update bandwidth if this service has higher speeds
					if circuit_id in circuit_bandwidth:
						current_dl, current_ul = circuit_bandwidth[circuit_id]
						if service_download > current_dl or service_upload > current_ul:
							# Update to use the maximum bandwidth
							new_download = max(service_download, current_dl)
							new_upload = max(service_upload, current_ul)
							circuit_bandwidth[circuit_id] = (new_download, new_upload)
							# Update the existing circuit node with new bandwidth
							for node in net.nodes:
								if node.id == circuit_id and node.type == NodeType.client:
									node.download = new_download
									node.upload = new_upload
									break
				else:
					# New customer+location combination, create circuit
					circuit_id = parentNodeIDCounter
					circuit_id_by_customer_location[circuit_key] = circuit_id
					circuit_bandwidth[circuit_id] = (service_download, service_upload)
					
					customer = NetworkNode(
						type=NodeType.client,
						id=circuit_id,
						parentId=None,
						displayName=customer_name,
						address=customer_name,
						customerName=customer_name,
						download=service_download,
						upload=service_upload
					)
					net.addRawNode(customer)
					parentNodeIDCounter = parentNodeIDCounter + 1
				
				# Create device node
				device = NetworkNode(
					id=device_counter,
					displayName=customer_name if customer_name else str(service["id"]),
					type=NodeType.device,
					parentId=circuit_id,
					mac=service["mac"],
					ipv4=ipv4,
					ipv6=ipv6
				)
				net.addRawNode(device)
				device_counter = device_counter + 1
				
				matched_via_alternate_method += 1
				if service["id"] not in service_ids_handled:
					service_ids_handled.append(service["id"])
	print("Matched " + "{:.0%}".format(len(service_ids_handled)/len(allServices)) + " of known services in Splynx.")
	print("Matched " + "{:.0%}".format(matched_via_primary_method/len(service_ids_handled)) + " services via primary method.")
	if matched_via_supplementation > 0:
		print("Supplemented " + "{:.0%}".format(matched_via_supplementation/matched_via_primary_method) + " of primary method services with additional IPs from CustomersOnline.")
	print("Matched " + "{:.0%}".format(matched_via_alternate_method/len(service_ids_handled)) + " services via alternate method.")
	print("Matched " + "{:.0%}".format(matched_with_parent_node/len(service_ids_handled)) + " of services found with their corresponding parent node.")
	
	# Report parent node assignment methods
	print("\nParent Node Assignment Methods:")
	total_assigned = sum(parent_assignment_methods.values()) - parent_assignment_methods['none']
	for method, count in parent_assignment_methods.items():
		if count > 0:
			if method == 'none':
				print(f"  No parent node: {count} ({count/sum(parent_assignment_methods.values()):.1%})")
			else:
				print(f"  {method}: {count} ({count/sum(parent_assignment_methods.values()):.1%})")
	
	if total_assigned > 0:
		print(f"\nTotal with parent nodes: {total_assigned} ({total_assigned/sum(parent_assignment_methods.values()):.1%})")
	
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
