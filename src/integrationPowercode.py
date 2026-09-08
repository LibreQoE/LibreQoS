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
from integrationCommon import NetworkGraph, NetworkNode, NodeType, apply_client_bandwidth_multiplier
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
			servicesDict[str(service['id'])] = {}
			servicesDict[str(service['id'])]['downloadMbps'] = apply_client_bandwidth_multiplier(int(service['rate_down']) / 1000)
			servicesDict[str(service['id'])]['uploadMbps'] = apply_client_bandwidth_multiplier(int(service['rate_up']) / 1000)
	return servicesDict

def getListSites():
	headers= {'Content-Type': 'application/x-www-form-urlencoded'}
	url = powercode_api_url() + ":444/api/preseem/index.php"
	data = {}
	data['apiKey'] = powercode_api_key()
	data['action'] = 'list_sites'

	r = requests.post(url, data=data, headers=headers, verify=False, timeout=10)
	return r.json()

def createShaper():
	net = NetworkGraph()
	requests.packages.urllib3.disable_warnings(category=InsecureRequestWarning)
	print("Fetching data from Powercode")

	customerInfo = getCustomerInfo()

	customerIDs = []
	for customer in customerInfo:
		if str(customer['id']) != '1':
			if str(customer['id']) != '':
				if customer['status'] == 'Active':
					customerIDint = int(customer['id'])
					if customerIDint != 0:
						if customerIDint != None:
							if customerIDint not in customerIDs:
								customerIDs.append(customerIDint)

	allServices = getListServices()
	sitesInfo = getListSites()

	# Map site infrastructure equipment (APs, routers, backhauls) by equipment id.
	# Powercode returns ids as ints or strings depending on endpoint, so str() everything.
	siteEquipment = {}
	siteNames = {}
	for site in sitesInfo:
		siteNodeId = "site_" + str(site['id'])
		siteNames[siteNodeId] = str(site['name']).strip() if site['name'] else site['name']
		for equipment in site['equipment']:
			siteEquipment[str(equipment['id'])] = {
				'siteNodeId': siteNodeId,
				'siteName': site['name'],
				'name': equipment['name'],
				'type': equipment['type']
			}

	acceptableEquipment = ['Customer Owned Equipment', 'Router', 'Customer Owned Equipment', 'Managed Routers', 'CPE']

	devicesByCustomerID = {}
	for customer in customerInfo:
		if customer['status'] == 'Active':
			chosenName = ''
			if customer['name'] != '':
				chosenName = str(customer['name']).strip()
			elif customer['company_name'] != '':
				chosenName = str(customer['company_name']).strip()
			else:
				chosenName = str(customer['id'])
			for equipment in customer['equipment']:
				if equipment['type'] in acceptableEquipment:
					if str(equipment['service_id']) in allServices:
						device = {}
						device['id'] = "c_" + str(customer['id']) + "_s_" + "_d_" + str(equipment['id'])
						device['name'] = equipment['name']
						device['ipv4'] = equipment['ip_address']
						device['mac'] = equipment['mac_address']
						parentEquipmentId = equipment.get('parent_id')
						if parentEquipmentId is not None and str(parentEquipmentId) in siteEquipment:
							device['parentEquipment'] = str(parentEquipmentId)
						else:
							device['parentEquipment'] = None
						if customer['id'] not in devicesByCustomerID:
							devicesByCustomerID[customer['id']] = {}
							devicesByCustomerID[customer['id']]['name'] = chosenName
						devicesByCustomerID[customer['id']]['downloadMbps'] = allServices[str(equipment['service_id'])]['downloadMbps']
						devicesByCustomerID[customer['id']]['uploadMbps'] = allServices[str(equipment['service_id'])]['uploadMbps']
						if 'devices' not in devicesByCustomerID[customer['id']]:
							devicesByCustomerID[customer['id']]['devices'] = []
						devicesByCustomerID[customer['id']]['devices'].append(device)

	# Resolve one topology parent (site AP) per customer: direct parent_id if it
	# maps to live site equipment, otherwise inherit from a sibling device that
	# resolves. Customers with no resolvable AP remain attached at the root.
	for customerID in devicesByCustomerID:
		resolvedParents = []
		for device in devicesByCustomerID[customerID]['devices']:
			if device['parentEquipment'] is not None:
				resolvedParents.append(device['parentEquipment'])
		devicesByCustomerID[customerID]['apEquipment'] = resolvedParents[0] if resolvedParents else None

	referencedEquipment = set()
	for customerID in devicesByCustomerID:
		apEquipment = devicesByCustomerID[customerID]['apEquipment']
		if apEquipment is not None:
			referencedEquipment.add(apEquipment)

	referencedSites = set()
	for equipmentId in referencedEquipment:
		referencedSites.add(siteEquipment[equipmentId]['siteNodeId'])

	# Display names must be trimmed and unique: the topology compiler resolves
	# parent references by (trimmed) name and validates exact matches.
	usedTopologyNames = set()
	def uniqueTopologyName(name):
		clean = str(name).strip() if name else ''
		if not clean:
			clean = 'Unnamed'
		base = clean
		counter = 2
		while clean in usedTopologyNames:
			clean = base + ' (' + str(counter) + ')'
			counter += 1
		usedTopologyNames.add(clean)
		return clean

	for siteNodeId in referencedSites:
		siteNode = NetworkNode(
			id=siteNodeId,
			displayName=uniqueTopologyName(siteNames[siteNodeId]),
			parentId='',
			type=NodeType.site,
			networkJsonId="powercode:" + siteNodeId
		)
		net.addRawNode(siteNode)

	for equipmentId in referencedEquipment:
		equipmentInfo = siteEquipment[equipmentId]
		apName = equipmentInfo['name'] if equipmentInfo['name'] else equipmentInfo['siteName']
		apNode = NetworkNode(
			id="ap_" + equipmentId,
			displayName=uniqueTopologyName(apName),
			parentId=equipmentInfo['siteNodeId'],
			type=NodeType.ap,
			networkJsonId="powercode:ap:" + equipmentId
		)
		net.addRawNode(apNode)

	attachedCount = 0
	for customerID in devicesByCustomerID:
		apEquipment = devicesByCustomerID[customerID]['apEquipment']
		clientParentId = ("ap_" + apEquipment) if apEquipment is not None else ''
		if apEquipment is not None:
			attachedCount += 1
		customer = NetworkNode(
				type=NodeType.client,
				id=str(customerID),
				parentId=clientParentId,
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
				parentId=str(customerID),
				mac=device["mac"],
				ipv4=[device['ipv4']],
				ipv6=[]
			)
			net.addRawNode(newDevice)
	print("Imported " + str(len(devicesByCustomerID)) + " customers")
	print("Topology: attached " + str(attachedCount) + " of " + str(len(devicesByCustomerID)) + " customers to " + str(len(referencedEquipment)) + " APs across " + str(len(referencedSites)) + " sites (" + str(len(devicesByCustomerID) - attachedCount) + " unattached)")
	net.prepareTree()
	net.materializeCompiledTopology("python/powercode", "full")

def importFromPowercode():
	#createNetworkJSON()
	createShaper()

if __name__ == '__main__':
	importFromPowercode()
