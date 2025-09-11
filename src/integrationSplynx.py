
from pythonCheck import checkPythonVersion
checkPythonVersion()

import requests
import warnings
import os
import csv
from liblqos_python import exclude_sites, find_ipv6_using_mikrotik, bandwidth_overhead_factor, splynx_api_key, \
	splynx_api_secret, splynx_api_url, splynx_strategy, overwrite_network_json_always

from integrationCommon import isIpv4Permitted
import base64
from requests.auth import HTTPBasicAuth
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType
import os
import csv

def build_online_ip_maps(customersOnline):
	"""
	Build maps of service_id -> [ipv4], [ipv6] from customers-online payload.
	"""
	ipv4sForService = {}
	ipv6sForService = {}
	for customerJson in customersOnline:
		ipv4 = customerJson.get('ipv4', '')
		ipv6 = customerJson.get('ipv6', '')
		service_id = customerJson.get('service_id')
		if service_id is None:
			continue
		# Only add non-empty IPv4
		if ipv4 and str(ipv4).strip():
			temp = ipv4sForService.get(service_id, [])
			if ipv4 not in temp:
				temp.append(ipv4)
			ipv4sForService[service_id] = temp
		# Only add non-empty IPv6
		if ipv6 and str(ipv6).strip():
			temp = ipv6sForService.get(service_id, [])
			if ipv6 not in temp:
				temp.append(ipv6)
			ipv6sForService[service_id] = temp
	return ipv4sForService, ipv6sForService

def supplement_existing_devices_with_online_ips(net, allServices, service_ids_handled, customersOnline, cust_id_to_name, allocated_ipv4s, allocated_ipv6s, device_by_service_id=None):
	"""
	For services already handled (devices created from static IPs), supplement missing
	IPv4/IPv6 from customers-online maps.
	Returns the number of services supplemented.
	"""
	ipv4sForService, ipv6sForService = build_online_ip_maps(customersOnline)
	matched_via_supplementation = 0
	for serviceItem in allServices:
		if serviceItem['id'] in service_ids_handled and serviceItem.get('status') == 'active':
			customer_id = serviceItem.get('customer_id')
			customer_name = cust_id_to_name.get(customer_id, str(customer_id))
			# Use index if provided; else scan nodes
			node = None
			if device_by_service_id and serviceItem['id'] in device_by_service_id:
				node = device_by_service_id[serviceItem['id']]
			else:
				for n in net.nodes:
					if (n.type == NodeType.device and n.displayName == customer_name and n.id >= 200000):
						node = n
						break
			if node is not None:
					needs_supplement = False
					supplemented_ipv4 = []
					supplemented_ipv6 = []
					# IPv4
					if ((not node.ipv4) or len(node.ipv4) == 0 or (len(node.ipv4) == 1 and not str(node.ipv4[0]).strip())) and serviceItem['id'] in ipv4sForService:
						for ipv4 in ipv4sForService[serviceItem['id']]:
							if ipv4 and str(ipv4).strip() and ipv4 not in allocated_ipv4s:
								supplemented_ipv4.append(ipv4)
								allocated_ipv4s[ipv4] = True
								needs_supplement = True
					# IPv6
					if ((not node.ipv6) or len(node.ipv6) == 0 or (len(node.ipv6) == 1 and not str(node.ipv6[0]).strip())) and serviceItem['id'] in ipv6sForService:
						for ipv6 in ipv6sForService[serviceItem['id']]:
							if ipv6 and str(ipv6).strip() and ipv6 not in allocated_ipv6s:
								supplemented_ipv6.append(ipv6)
								allocated_ipv6s[ipv6] = True
								needs_supplement = True
					if needs_supplement:
						if supplemented_ipv4:
							node.ipv4 = supplemented_ipv4
						if supplemented_ipv6:
							node.ipv6 = supplemented_ipv6
						matched_via_supplementation += 1
	return matched_via_supplementation

def create_devices_from_online_for_unhandled_services(net, allServices, service_ids_handled, customersOnline, cust_id_to_name, downloadForTariffID, uploadForTariffID, device_counter, allocated_ipv4s, allocated_ipv6s, parent_selector=None, device_by_service_id=None):
	"""
	For services that didn't produce devices via static IPs, create devices for those
	with online IPs. Optionally select a parent node via parent_selector(serviceItem).
	Returns the number of services created via this alternate method.
	"""
	ipv4sForService, ipv6sForService = build_online_ip_maps(customersOnline)
	matched_via_alternate_method = 0
	for service in allServices:
		if service['id'] not in service_ids_handled and service.get('status') == 'active':
			ipv4 = [ip for ip in ipv4sForService.get(service['id'], []) if ip and str(ip).strip()]
			ipv6 = [ip for ip in ipv6sForService.get(service['id'], []) if ip and str(ip).strip()]
			if not ipv4 and not ipv6:
				continue
			customer_id = service.get('customer_id', service['id'])
			customer_name = cust_id_to_name.get(customer_id, str(customer_id))
			# Speeds
			service_download = downloadForTariffID[service['tariff_id']]
			service_upload = uploadForTariffID[service['tariff_id']]
			circuit_id = service['id']
			# Determine parent for the client circuit (AP/Site) if selector provided
			parent_id = None
			if parent_selector is not None:
				try:
					parent_id = parent_selector(service)
				except Exception:
					parent_id = None
			# Create customer circuit node
			customer = NetworkNode(
				type=NodeType.client,
				id=circuit_id,
				parentId=parent_id,
				displayName=customer_name,
				address=customer_name,
				customerName=customer_name,
				download=service_download,
				upload=service_upload
			)
			net.addRawNode(customer)
			# Create device node under the client circuit
			device = NetworkNode(
				id=device_counter[0],
				displayName=customer_name,
				type=NodeType.device,
				parentId=circuit_id,
				mac=service.get('mac', ''),
				ipv4=ipv4,
				ipv6=ipv6
			)
			net.addRawNode(device)
			if device_by_service_id is not None:
				device_by_service_id[service['id']] = device
			device_counter[0] += 1
			service_ids_handled.append(service['id'])
			matched_via_alternate_method += 1
	return matched_via_alternate_method

def run_splynx_pipeline(strategy_name: str):
	"""
	Unified pipeline to fetch data, build infrastructure, create static clients,
	supplement with customers-online, and finalize outputs.
	"""
	net = NetworkGraph()
	print(f"Using {strategy_name.upper()} strategy - unified pipeline")
	print("Fetching data from Spylnx")
	headers = buildHeaders()
	print("Fetching tariffs from Spylnx")
	tariff, downloadForTariffID, uploadForTariffID = getTariffs(headers)
	print("Fetching all customers from Spylnx")
	customers = getCustomers(headers)
	print("Fetching online customers from Spylnx")
	customersOnline = getCustomersOnline(headers)

	ipForRouter = {}
	nameForRouterID = {}
	routerIdList = []
	sectorForRouter = {}
	monitoring = []
	siteBandwidth = buildSiteBandwidths()
	allServices = []

	if strategy_name in ("ap_only", "ap_site", "full"):
		print("Fetching routers from Spylnx")
		ipForRouter, nameForRouterID, routerIdList = getRouters(headers)
		print("Fetching sectors from Spylnx")
		sectorForRouter = getSectors(headers)
		print("Fetching services from Spylnx")
		allServices = getAllServices(headers)
		print("Fetching hardware monitoring from Spylnx")
		monitoring = getMonitoring(headers)
	else:
		print("Fetching services from Spylnx")
		allServices = getAllServices(headers)

	print("Successfully fetched data from Spylnx")
	# Build basic customer map
	cust_id_to_name = {c['id']: c['name'] for c in customers}

	# Build hardware maps for parent selection
	hardware_name = {}
	hardware_parent = {}
	hardware_type = {}
	hardware_name_extended = {}
	ap_nodes = {}
	if monitoring:
		for dev in monitoring:
			hardware_name[dev['id']] = dev['title']
			if 'parent_id' in dev:
				hardware_parent[dev['id']] = dev['parent_id']
			if 'type' in dev:
				if dev['type'] == 5:
					hardware_type[dev['id']] = 'AP'
					ap_nodes[dev['id']] = dev
				else:
					hardware_type[dev['id']] = 'Site'
		for dev in monitoring:
			if 'parent_id' in dev and dev['id'] in hardware_parent and hardware_parent[dev['id']] in hardware_name:
				hardware_name_extended[dev['id']] = hardware_name[hardware_parent[dev['id']]] + "_" + dev['title']
			if dev['id'] not in hardware_name_extended:
				hardware_name_extended[dev['id']] = dev['title']

	# Infrastructure builder
	def build_infrastructure():
		if strategy_name == 'flat':
			return
		if strategy_name == 'ap_only':
			print(f"Creating {len(ap_nodes)} AP nodes")
			for ap_id, ap_device in ap_nodes.items():
				download = 10000
				upload = 10000
				nodeName = ap_device['title']
				if nodeName in siteBandwidth:
					download = siteBandwidth[nodeName]["download"]
					upload = siteBandwidth[nodeName]["upload"]
				node = NetworkNode(id=ap_id, displayName=nodeName, type=NodeType.ap, parentId=None, download=download, upload=upload, address=None)
				net.addRawNode(node)
			return
		# ap_site and full
		print("Creating site and AP infrastructure")
		createInfrastructureNodes(net, monitoring, hardware_name, hardware_parent, hardware_type, siteBandwidth, hardware_name_extended)

	# Parent selector per strategy
	def select_parent(serviceItem):
		if strategy_name == 'flat':
			return None
		parent_node_id, _ = findBestParentNode(serviceItem, hardware_name, ipForRouter, sectorForRouter)
		if strategy_name == 'ap_only':
			return parent_node_id if (parent_node_id in ap_nodes) else None
		return parent_node_id

	# Run pipeline
	build_infrastructure()
	allocated_ipv4s = {}
	allocated_ipv6s = {}
	device_counter = [200000]
	service_ids_handled = []
	device_by_service_id = {}
	static_created = 0
	for serviceItem in allServices:
		if serviceItem.get('status') == 'active':
			ipv4_list, ipv6_list = extractServiceIPs(serviceItem, cust_id_to_name, allocated_ipv4s, allocated_ipv6s)
			if ipv4_list or ipv6_list:
				parent_node_id = select_parent(serviceItem)
				circuit_id = createClientAndDevice(
					net, serviceItem, cust_id_to_name, downloadForTariffID, uploadForTariffID, device_counter, parent_node_id, ipv4_list, ipv6_list
				)
				service_ids_handled.append(serviceItem['id'])
				# Last added node is the device
				if net.nodes:
					device_by_service_id[serviceItem['id']] = net.nodes[-1]
				static_created += 1

	# Supplement and fallback using customers-online
	matched_via_supplementation = supplement_existing_devices_with_online_ips(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name, allocated_ipv4s, allocated_ipv6s, device_by_service_id=device_by_service_id
	)
	matched_via_alternate_method = create_devices_from_online_for_unhandled_services(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name,
		downloadForTariffID, uploadForTariffID, device_counter, allocated_ipv4s, allocated_ipv6s,
		parent_selector=select_parent, device_by_service_id=device_by_service_id
	)

	# Counters
	print(f"Services (active): {sum(1 for s in allServices if s.get('status')=='active')} | Online sessions: {len(customersOnline)}")
	print(f"Static devices: {static_created} | Supplemented: {matched_via_supplementation} | Fallback-created: {matched_via_alternate_method}")
	print(f"Total client entries: {len(service_ids_handled)}")

	# Finalize
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

def extractServiceIPs(serviceItem, cust_id_to_name, allocated_ipv4s, allocated_ipv6s):
	"""
	Extract IPv4 and IPv6 addresses from service item, handling duplicates.
	"""
	ipv4_list = []
	ipv6_list = []
	
	# Add primary IPv4
	if serviceItem['ipv4'] != '':
		if serviceItem['ipv4'] not in allocated_ipv4s:
			ipv4_list.append(serviceItem['ipv4'])
			allocated_ipv4s[serviceItem['ipv4']] = True
		else:
			print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IP of " + serviceItem['ipv4'] + ". IP omitted.")
	
	# Add IPv4 routes (additional IPs/subnets)
	if 'ipv4_route' in serviceItem and serviceItem['ipv4_route']:
		for ipv4_route in serviceItem['ipv4_route'].split(', '):
			if ipv4_route and ipv4_route.strip():
				if ipv4_route not in allocated_ipv4s:
					ipv4_list.append(ipv4_route)
					allocated_ipv4s[ipv4_route] = True
				else:
					print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IPv4 route of " + ipv4_route + ". IP omitted.")
	
	# Add primary IPv6
	if serviceItem['ipv6'] != '':
		if serviceItem['ipv6'] not in allocated_ipv6s:
			ipv6_list.append(serviceItem['ipv6'])
			allocated_ipv6s[serviceItem['ipv6']] = True
		else:
			print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IP of " + serviceItem['ipv6'] + ". IP omitted.")
	
	# Add IPv6 delegated prefixes
	if 'ipv6_delegated' in serviceItem and serviceItem['ipv6_delegated']:
		for ipv6_delegation in serviceItem['ipv6_delegated'].split(', '):
			if ipv6_delegation and ipv6_delegation.strip():
				if ipv6_delegation not in allocated_ipv6s:
					ipv6_list.append(ipv6_delegation)
					allocated_ipv6s[ipv6_delegation] = True
				else:
					print("Client " + cust_id_to_name[serviceItem['customer_id']] + " had duplicate IPv6 delegation of " + ipv6_delegation + ". IP omitted.")
	
	return ipv4_list, ipv6_list

def createClientAndDevice(net, serviceItem, cust_id_to_name, downloadForTariffID, uploadForTariffID, device_counter, parent_node_id, ipv4_list, ipv6_list):
	"""
	Create client and device nodes for a service.
	"""
	customer_id = serviceItem['customer_id']
	customer_name = cust_id_to_name.get(customer_id, str(customer_id))
	
	# Get service bandwidth
	service_download = downloadForTariffID[serviceItem['tariff_id']]
	service_upload = uploadForTariffID[serviceItem['tariff_id']]
	
	# Use service ID as unique circuit ID to prevent merging services with different speed plans
	circuit_id = serviceItem['id']
	
	# Create the customer circuit node for each service
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
	
	# Always create a device for each service
	device = NetworkNode(
		id=device_counter[0],
		displayName=customer_name,
		type=NodeType.device,
		parentId=circuit_id,
		mac=serviceItem['mac'],
		ipv4=ipv4_list,
		ipv6=ipv6_list
	)
	net.addRawNode(device)
	device_counter[0] += 1
	
	return circuit_id

def createInfrastructureNodes(net, monitoring, hardware_name, hardware_parent, hardware_type, siteBandwidth, hardware_name_extended):
	"""
	Create site and AP nodes from monitoring data.
	"""
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

def findBestParentNode(serviceItem, hardware_name, ipForRouter, sectorForRouter):
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
	
	return parent_node_id, assignment_method

def createShaper():
	"""
	Main function to route to appropriate strategy based on configuration.
	"""
	try:
		strategy = splynx_strategy().lower()
		print(f"Using Splynx strategy: {strategy}")
		# Unified pipeline for all strategies
		if strategy in ("flat", "ap_only", "ap_site", "full"):
			run_splynx_pipeline(strategy)
		else:
			print(f"Unknown strategy '{strategy}', defaulting to ap_only")
			run_splynx_pipeline('ap_only')
	except Exception as e:
		print(f"Error reading strategy config, defaulting to ap_only: {e}")
		run_splynx_pipeline('ap_only')

def createShaperFull():
	# Deprecated wrapper: use unified pipeline
	run_splynx_pipeline('full')

def createShaperFlat():
	# Deprecated wrapper: use unified pipeline
	run_splynx_pipeline('flat')

def createShaperApOnly():
	"""
	AP-only strategy: Single layer of AP + Clients (similar to Preseem).
	Optimal balance between hierarchy and performance - should be default.
	"""
	# Deprecated body retained below for reference; not used by entrypoint
	net = NetworkGraph()
	
	print("Using AP_ONLY strategy - single layer AP + Clients topology")
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
	siteBandwidth = buildSiteBandwidths()
	print("Successfully fetched data from Spylnx")
	cust_id_to_name = {}
	for customer in customers:
		cust_id_to_name[customer['id']] = customer['name']
	
	# Build hardware info but only use APs, not sites
	hardware_name = {}
	hardware_parent = {}
	hardware_type = {}
	ap_nodes = {}
	
	for monitored_device in monitoring:
		hardware_name[monitored_device['id']] = monitored_device['title']
		if 'parent_id' in monitored_device:
			hardware_parent[monitored_device['id']] = monitored_device['parent_id']
		if 'type' in monitored_device:
			if monitored_device['type'] == 5:  # AP type
				hardware_type[monitored_device['id']] = 'AP'
				ap_nodes[monitored_device['id']] = monitored_device
			else:
				hardware_type[monitored_device['id']] = 'Site'
	
	# Create only AP nodes (no sites), all as root nodes
	print(f"Creating {len(ap_nodes)} AP nodes")
	for ap_id, ap_device in ap_nodes.items():
		download = 10000
		upload = 10000
		nodeName = ap_device['title']
		if nodeName in siteBandwidth:
			download = siteBandwidth[nodeName]["download"]
			upload = siteBandwidth[nodeName]["upload"]
		
		# In ap_only strategy, APs have no parent (parentId=None)
		node = NetworkNode(
			id=ap_id, 
			displayName=nodeName, 
			type=NodeType.ap,
			parentId=None,  # APs are root nodes
			download=download, 
			upload=upload, 
			address=None
		)
		net.addRawNode(node)
	
	service_ids_handled = []
	allocated_ipv4s = {}
	allocated_ipv6s = {}
	device_counter = [200000]
	
	print("Assigning clients to APs")
	for serviceItem in allServices:
		if serviceItem['status'] == 'active':
			ipv4_list, ipv6_list = extractServiceIPs(serviceItem, cust_id_to_name, allocated_ipv4s, allocated_ipv6s)
			
			if ipv4_list or ipv6_list:
				# Find AP parent (only APs, not sites)
				parent_node_id, assignment_method = findBestParentNode(
					serviceItem, hardware_name, ipForRouter, sectorForRouter
				)
				
				# Only allow AP parents in ap_only strategy
				if parent_node_id and parent_node_id in ap_nodes:
					# Valid AP parent
					pass
				else:
					# No valid AP parent found, assign to None (root level)
					parent_node_id = None
				
				circuit_id = createClientAndDevice(
					net, serviceItem, cust_id_to_name, downloadForTariffID, 
					uploadForTariffID, device_counter, parent_node_id, ipv4_list, ipv6_list
				)
				service_ids_handled.append(serviceItem['id'])
	
	# After initial static-IP creation, supplement and fallback from customers-online
	def ap_only_parent_selector(serviceItem):
		# Use existing parent finder, but only accept AP parents; otherwise None
		parent_node_id, _ = findBestParentNode(serviceItem, hardware_name, ipForRouter, sectorForRouter)
		return parent_node_id if (parent_node_id in ap_nodes) else None
	matched_via_supplementation = supplement_existing_devices_with_online_ips(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name, allocated_ipv4s, allocated_ipv6s
	)
	matched_via_alternate_method = create_devices_from_online_for_unhandled_services(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name,
		downloadForTariffID, uploadForTariffID, device_counter, allocated_ipv4s, allocated_ipv6s,
		parent_selector=ap_only_parent_selector
	)
	print(f"Created {len(service_ids_handled)} client entries assigned to APs (including online supplementation: {matched_via_supplementation}, fallback: {matched_via_alternate_method})")
	
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

def createShaperApSite():
	"""
	AP-Site strategy: Site → AP → Clients structure.
	Moderate hierarchy providing site-level aggregation.
	"""
	# Deprecated body retained below for reference; not used by entrypoint
	net = NetworkGraph()
	
	print("Using AP_SITE strategy - Site → AP → Clients topology")
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
	siteBandwidth = buildSiteBandwidths()
	print("Successfully fetched data from Spylnx")
	
	cust_id_to_name = {}
	for customer in customers:
		cust_id_to_name[customer['id']] = customer['name']
	
	# Build hardware info with site → AP hierarchy
	hardware_name = {}
	hardware_parent = {}
	hardware_type = {}
	hardware_name_extended = {}
	
	for monitored_device in monitoring:
		hardware_name[monitored_device['id']] = monitored_device['title']
		if 'parent_id' in monitored_device:
			hardware_parent[monitored_device['id']] = monitored_device['parent_id']
		if 'type' in monitored_device:
			if monitored_device['type'] == 5:
				hardware_type[monitored_device['id']] = 'AP'
			else:
				hardware_type[monitored_device['id']] = 'Site'
	
	# Build extended names with parent hierarchy
	for monitored_device in monitoring:
		if 'parent_id' in monitored_device:
			if monitored_device['id'] in hardware_parent:
				if hardware_parent[monitored_device['id']] in hardware_name:
					hardware_name_extended[monitored_device['id']] = hardware_name[hardware_parent[monitored_device['id']]] + "_" + monitored_device['title']
		if monitored_device['id'] not in hardware_name_extended:
			hardware_name_extended[monitored_device['id']] = monitored_device['title']
	
	# Create infrastructure nodes with proper site → AP hierarchy
	print("Creating site and AP infrastructure")
	createInfrastructureNodes(net, monitoring, hardware_name, hardware_parent, hardware_type, siteBandwidth, hardware_name_extended)
	
	service_ids_handled = []
	allocated_ipv4s = {}
	allocated_ipv6s = {}
	device_counter = [200000]
	
	print("Assigning clients to infrastructure")
	for serviceItem in allServices:
		if serviceItem['status'] == 'active':
			ipv4_list, ipv6_list = extractServiceIPs(serviceItem, cust_id_to_name, allocated_ipv4s, allocated_ipv6s)
			
			if ipv4_list or ipv6_list:
				parent_node_id, assignment_method = findBestParentNode(
					serviceItem, hardware_name, ipForRouter, sectorForRouter
				)
				
				circuit_id = createClientAndDevice(
					net, serviceItem, cust_id_to_name, downloadForTariffID, 
					uploadForTariffID, device_counter, parent_node_id, ipv4_list, ipv6_list
				)
				service_ids_handled.append(serviceItem['id'])
	
	# After initial static-IP creation, supplement and fallback from customers-online
	def ap_site_parent_selector(serviceItem):
		parent_node_id, _ = findBestParentNode(serviceItem, hardware_name, ipForRouter, sectorForRouter)
		return parent_node_id
	matched_via_supplementation = supplement_existing_devices_with_online_ips(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name, allocated_ipv4s, allocated_ipv6s
	)
	matched_via_alternate_method = create_devices_from_online_for_unhandled_services(
		net, allServices, service_ids_handled, customersOnline, cust_id_to_name,
		downloadForTariffID, uploadForTariffID, device_counter, allocated_ipv4s, allocated_ipv6s,
		parent_selector=ap_site_parent_selector
	)
	print(f"Created {len(service_ids_handled)} client entries with site → AP hierarchy (including online supplementation: {matched_via_supplementation}, fallback: {matched_via_alternate_method})")
	
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

# Lightweight wrappers to avoid confusion; unified pipeline is canonical
def createShaperApOnly():
	return run_splynx_pipeline('ap_only')

def createShaperApSite():
	return run_splynx_pipeline('ap_site')

def createShaperFull():
	return run_splynx_pipeline('full')

def createShaperFlat():
	return run_splynx_pipeline('flat')

if __name__ == '__main__':
	importFromSplynx()
