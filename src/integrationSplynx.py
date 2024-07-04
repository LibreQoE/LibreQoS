from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import warnings
from ispConfig import excludeSites, findIPv6usingMikrotik, bandwidthOverheadFactor, exceptionCPEs, splynx_api_key, splynx_api_secret, splynx_api_url
from integrationCommon import isIpv4Permitted
import base64
from requests.auth import HTTPBasicAuth
if findIPv6usingMikrotik == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType

def buildHeaders():
	credentials = splynx_api_key + ':' + splynx_api_secret
	credentials = base64.b64encode(credentials.encode()).decode()
	return {'Authorization' : "Basic %s" % credentials}

def spylnxRequest(target, headers):
	# Sends a REST GET request to Spylnx and returns the
	# result in JSON
	url = splynx_api_url + "/api/2.0/" + target
	r = requests.get(url, headers=headers, timeout=120)
	return r.json()

def getTariffs(headers):
	data = spylnxRequest("admin/tariffs/internet", headers)
	tariff = []
	downloadForTariffID = {}
	uploadForTariffID = {}
	for tariff in data:
		tariffID = tariff['id']
		speed_download = round((int(tariff['speed_download']) / 1000))
		speed_upload = round((int(tariff['speed_upload']) / 1000))
		downloadForTariffID[tariffID] = speed_download
		uploadForTariffID[tariffID] = speed_upload
	return (tariff, downloadForTariffID, uploadForTariffID)

def getCustomers(headers):
	data = spylnxRequest("admin/customers/customer", headers)
	#addressForCustomerID = {}
	#customerIDs = []
	#for customer in data:
	#	customerIDs.append(customer['id'])
	#	addressForCustomerID[customer['id']] = customer['street_1']
	return data

def getRouters(headers):
	data = spylnxRequest("admin/networking/routers", headers)
	ipForRouter = {}
	for router in data:
		routerID = router['id']
		ipForRouter[routerID] = router['ip']
	print("Router IPs found: " + str(len(ipForRouter)))
	return ipForRouter

def combineAddress(json):
	# Combines address fields into a single string
	# The API docs seem to indicate that there isn't a "state" field?
	if json["street_1"]=="" and json["city"]=="" and json["zip_code"]=="":
		return str(json["id"]) + "/" + json["name"]
	else:
		return json["street_1"] + " " + json["city"] + " " + json["zip_code"]

def getAllServices(headers):
	services = spylnxRequest("admin/customers/customer/0/internet-services?main_attributes%5Bstatus%5D=active", headers)
	return services

def getAllIPs(headers):
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
	net = NetworkGraph()

	print("Fetching data from Spylnx")
	headers = buildHeaders()
	tariff, downloadForTariffID, uploadForTariffID = getTariffs(headers)
	customers = getCustomers(headers)
	ipForRouter = getRouters(headers)
	allServices = getAllServices(headers)
	ipv4ByCustomerID, ipv6ByCustomerID = getAllIPs(headers)
	
	allServicesDict = {}
	for serviceItem in allServices:
		if (serviceItem['status'] == 'active'):
			if serviceItem["customer_id"] not in allServicesDict:
				allServicesDict[serviceItem["customer_id"]] = []
			temp = allServicesDict[serviceItem["customer_id"]]
			temp.append(serviceItem)
			allServicesDict[serviceItem["customer_id"]] = temp
	
	#It's not very clear how a service is meant to handle multiple
	#devices on a shared tariff. Creating each service as a combined
	#entity including the customer, to be on the safe side.
	for customerJson in customers:
		if customerJson['status'] == 'active':
			if customerJson['id'] in allServicesDict:
				servicesForCustomer = allServicesDict[customerJson['id']]
				for service in servicesForCustomer:
					combinedId = "c_" + str(customerJson["id"]) + "_s_" + str(service["id"])
					tariff_id = service['tariff_id']
					customer = NetworkNode(
						type=NodeType.client,
						id=combinedId,
						displayName=customerJson["name"],
						address=combineAddress(customerJson),
						customerName=customerJson["name"],
						download=downloadForTariffID[tariff_id],
						upload=uploadForTariffID[tariff_id],
					)
					net.addRawNode(customer)
					
					ipv4 = []
					ipv6 = []
					routerID = service['router_id']
					
					# If not "Taking IPv4" (Router will assign IP), then use router's set IP
					taking_ipv4 = int(service['taking_ipv4'])
					if taking_ipv4 == 0:
						if routerID in ipForRouter:
								ipv4 = [ipForRouter[routerID]]

					elif taking_ipv4 == 1:
						ipv4 = [service['ipv4']]
					if len(ipv4) == 0:
						#Only do this if single service for a customer
						if len(servicesForCustomer) == 1:
							if customerJson['id'] in ipv4ByCustomerID:
								ipv4 = ipv4ByCustomerID[customerJson['id']]
						
					# If not "Taking IPv6" (Router will assign IP), then use router's set IP
					if isinstance(service['taking_ipv6'], str):
						taking_ipv6 = int(service['taking_ipv6'])
					else:
						taking_ipv6 = service['taking_ipv6']
					if taking_ipv6 == 0:
						ipv6 = []
					elif taking_ipv6 == 1:
						ipv6 = [service['ipv6']]
					
					device = NetworkNode(
						id=combinedId+"_d" + str(service["id"]),
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
	#createNetworkJSON()
	createShaper()

if __name__ == '__main__':
	importFromSplynx()
