from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import os
import csv
from ispConfig import uispSite, uispStrategy
from integrationCommon import isIpv4Permitted, fixSubnet

def uispRequest(target):
	# Sends an HTTP request to UISP and returns the
	# result in JSON. You only need to specify the
	# tail end of the URL, e.g. "sites"
	from ispConfig import UISPbaseURL, uispAuthToken
	url = UISPbaseURL + "/nms/api/v2.1/" + target
	headers = {'accept': 'application/json', 'x-auth-token': uispAuthToken}
	r = requests.get(url, headers=headers, timeout=10)
	return r.json()

def buildFlatGraph():
	# Builds a high-performance (but lacking in site or AP bandwidth control)
	# network.
	from integrationCommon import NetworkGraph, NetworkNode, NodeType
	from ispConfig import generatedPNUploadMbps, generatedPNDownloadMbps

	# Load network sites
	print("Loading Data from UISP")
	sites = uispRequest("sites")
	devices = uispRequest("devices?withInterfaces=true&authorized=true")

	# Build a basic network adding every client to the tree
	print("Building Flat Topology")
	net = NetworkGraph()

	for site in sites:
		type = site['identification']['type']
		if type == "endpoint":
			id = site['identification']['id']
			address = site['description']['address']
			customerName = ''
			name = site['identification']['name']
			type = site['identification']['type']
			download = generatedPNDownloadMbps
			upload = generatedPNUploadMbps
			if (site['qos']['downloadSpeed']) and (site['qos']['uploadSpeed']):
				download = int(round(site['qos']['downloadSpeed']/1000000))
				upload = int(round(site['qos']['uploadSpeed']/1000000))

			node = NetworkNode(id=id, displayName=name, type=NodeType.client, download=download, upload=upload, address=address, customerName=customerName)
			net.addRawNode(node)
			for device in devices:
				if device['identification']['site'] is not None and device['identification']['site']['id'] == id:
					# The device is at this site, so add it
					ipv4 = []
					ipv6 = []

					for interface in device["interfaces"]:
						for ip in interface["addresses"]:
							ip = ip["cidr"]
							if isIpv4Permitted(ip):
								ip = fixSubnet(ip)
								if ip not in ipv4:
									ipv4.append(ip)

					# TODO: Figure out Mikrotik IPv6?
					mac = device['identification']['mac']

					net.addRawNode(NetworkNode(id=device['identification']['id'], displayName=device['identification']
						['name'], parentId=id, type=NodeType.device, ipv4=ipv4, ipv6=ipv6, mac=mac))

	# Finish up
	net.prepareTree()
	net.plotNetworkGraph(False)
	if net.doesNetworkJsonExist():
		print("network.json already exists. Leaving in-place.")
	else:
		net.createNetworkJson()
	net.createShapedDevices()

def buildFullGraph():
	# Attempts to build a full network graph, incorporating as much of the UISP
	# hierarchy as possible.
	from integrationCommon import NetworkGraph, NetworkNode, NodeType
	from ispConfig import generatedPNUploadMbps, generatedPNDownloadMbps

	# Load network sites
	print("Loading Data from UISP")
	sites = uispRequest("sites")
	devices = uispRequest("devices?withInterfaces=true&authorized=true")
	dataLinks = uispRequest("data-links?siteLinksOnly=true")

	# Do we already have a integrationUISPbandwidths.csv file?
	siteBandwidth = {}
	if os.path.isfile("integrationUISPbandwidths.csv"):
		with open('integrationUISPbandwidths.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				name, download, upload = row
				download = int(download)
				upload = int(upload)
				siteBandwidth[name] = {"download": download, "upload": upload}
	
	# Find AP capacities from UISP
	for device in devices:
		if device['identification']['role'] == "ap":
			name = device['identification']['name']
			if not name in siteBandwidth and device['overview']['downlinkCapacity'] and device['overview']['uplinkCapacity']:
				download = int(device['overview']
							   ['downlinkCapacity'] / 1000000)
				upload = int(device['overview']['uplinkCapacity'] / 1000000)
				siteBandwidth[device['identification']['name']] = {
					"download": download, "upload": upload}
	
	# Find Site Capacities by AirFiber capacities
	foundAirFibersBySite = {}
	for device in devices:
		if device['identification']['site']['type'] == 'site':
			if device['identification']['role'] == "station":
				if device['identification']['type'] == "airFiber":
					if device['overview']['status'] == 'active':
						download = int(device['overview']['downlinkCapacity']/ 1000000)
						upload = int(device['overview']['uplinkCapacity']/ 1000000)
						# Make sure to use half of reported bandwidth for AF60-LRs
						if device['identification']['model'] == "AF60-LR":
							download = int(download / 2)
							upload = int(download / 2)
						if device['identification']['site']['id'] in foundAirFibersBySite:
							if (download > foundAirFibersBySite['download']) or (upload > foundAirFibersBySite['upload']):
								foundAirFibersBySite[device['identification']['site']['id']]['download'] = download
								foundAirFibersBySite[device['identification']['site']['id']]['upload'] = upload
						else:
							foundAirFibersBySite[device['identification']['site']['id']] = {'download': download, 'upload': upload}
	
	print("Building Topology")
	net = NetworkGraph()
	# Add all sites and client sites
	for site in sites:
		id = site['identification']['id']
		name = site['identification']['name']
		type = site['identification']['type']
		download = generatedPNDownloadMbps
		upload = generatedPNUploadMbps
		address = ""
		customerName = ""
		if site['identification']['parent'] is None:
			parent = ""
		else:
			parent = site['identification']['parent']['id']
		match type:
			case "site":
				nodeType = NodeType.site
				if name in siteBandwidth:
					# Use the CSV bandwidth values
					download = siteBandwidth[name]["download"]
					upload = siteBandwidth[name]["upload"]
				elif id in foundAirFibersBySite:
					download = foundAirFibersBySite[id]['download']
					upload = foundAirFibersBySite[id]['upload']
				else:
					# Add them just in case
					siteBandwidth[name] = {
						"download": download, "upload": upload}
			case default:
				nodeType = NodeType.client
				address = site['description']['address']
				try:
					customerName = site["ucrm"]["client"]["name"]
				except:
					customerName = ""
				if (site['qos']['downloadSpeed']) and (site['qos']['uploadSpeed']):
					download = int(round(site['qos']['downloadSpeed']/1000000))
					upload = int(round(site['qos']['uploadSpeed']/1000000))

		node = NetworkNode(id=id, displayName=name, type=nodeType,
						   parentId=parent, download=download, upload=upload, address=address, customerName=customerName)
		# If this is the uispSite node, it becomes the root. Otherwise, add it to the
		# node soup.
		if name == uispSite:
			net.replaceRootNote(node)
		else:
			net.addRawNode(node)

		for device in devices:
			if device['identification']['site'] is not None and device['identification']['site']['id'] == id:
				# The device is at this site, so add it
				ipv4 = []
				ipv6 = []

				for interface in device["interfaces"]:
					for ip in interface["addresses"]:
						ip = ip["cidr"]
						if isIpv4Permitted(ip):
							ip = fixSubnet(ip)
							if ip not in ipv4:
								ipv4.append(ip)

				# TODO: Figure out Mikrotik IPv6?
				mac = device['identification']['mac']

				net.addRawNode(NetworkNode(id=device['identification']['id'], displayName=device['identification']
							   ['name'], parentId=id, type=NodeType.device, ipv4=ipv4, ipv6=ipv6, mac=mac))

	# Now iterate access points, and look for connections to sites
	for node in net.nodes:
		if node.type == NodeType.device:
			for dl in dataLinks:
				if dl['from']['device'] is not None and dl['from']['device']['identification']['id'] == node.id:
					if dl['to']['site'] is not None and dl['from']['site']['identification']['id'] != dl['to']['site']['identification']['id']:
						target = net.findNodeIndexById(
							dl['to']['site']['identification']['id'])
						if target > -1:
							# We found the site
							if net.nodes[target].type == NodeType.client or net.nodes[target].type == NodeType.clientWithChildren:
								net.nodes[target].parentId = node.id
								node.type = NodeType.ap
								if node.displayName in siteBandwidth:
									# Use the bandwidth numbers from the CSV file
									node.uploadMbps = siteBandwidth[node.displayName]["upload"]
									node.downloadMbps = siteBandwidth[node.displayName]["download"]
								else:
									# Add some defaults in case they want to change them
									siteBandwidth[node.displayName] = {
										"download": generatedPNDownloadMbps, "upload": generatedPNUploadMbps}

	net.prepareTree()
	net.plotNetworkGraph(False)
	if net.doesNetworkJsonExist():
		print("network.json already exists. Leaving in-place.")
	else:
		net.createNetworkJson()
	net.createShapedDevices()

	# Save integrationUISPbandwidths.csv
	# (the newLine fixes generating extra blank lines)
	# Saves as .template as to not overwrite
	with open('integrationUISPbandwidths.template.csv', 'w', newline='') as csvfile:
		wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
		wr.writerow(['ParentNode', 'Download Mbps', 'Upload Mbps'])
		for device in siteBandwidth:
			entry = (
				device, siteBandwidth[device]["download"], siteBandwidth[device]["upload"])
			wr.writerow(entry)


def importFromUISP():
	match uispStrategy:
		case "full": buildFullGraph()
		case default: buildFlatGraph()


if __name__ == '__main__':
	importFromUISP()
