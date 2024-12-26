
from pythonCheck import checkPythonVersion
checkPythonVersion()

import requests
import warnings
import os
import csv
from liblqos_python import exclude_sites, find_ipv6_using_mikrotik, bandwidth_overhead_factor, overwrite_network_json_always
from integrationCommon import isIpv4Permitted
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType
import os
import csv

def importBandwidthOverrides():
	"""
	Build a dictionary of site bandwidths by reading data from a CSV file.
	"""
	siteBandwidth = {}
	if os.path.isfile("integrationCustomBandwidths.csv"):
		with open('integrationCustomBandwidths.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				name, download, upload = row
				download = int(float(download))
				upload = int(float(upload))
				siteBandwidth[name] = {"download": download, "upload": upload}
	return siteBandwidth

def createShaper():
	"""
	Main function to fetch data from Custom, build the network graph, and shape devices.
	"""
	net = NetworkGraph()
	parentNodeIDCounter = 100000
	# Pull in bandwidth overrides dictionary from integrationCustomBandwidths.csv
	siteBandwidthOverride = importBandwidthOverrides()	
	
	# Create sites
	for site_item in your_site_list:			# Iterate through a site list you've created somewhere
		parent_id = None						# No parent id by default, but you can add specify it later, prior to the line node = NetworkNode
		download = 10000						# Default speed is 10G, but you can add specify it later, prior to the line node = NetworkNode
		upload = 10000							# Default speed is 10G, but you can add specify it later, prior to the line node = NetworkNode
		if nodeName in siteBandwidthOverride:
			download = siteBandwidthOverride[nodeName]["download"]
			upload = siteBandwidthOverride[nodeName]["upload"]
		node = NetworkNode(id=site_item['id'], displayName=site_item['name'], type=NodeType.site,
						   parentId=parent_id, download=download, upload=upload, address=None)
		net.addRawNode(node)
	
	# Create subscriber sites and devices
	for serviceItem in your_list_of_service:	# Iterate through a service/customer list you've created somewhere
		customer = NetworkNode(
			type=NodeType.client,
			id=parentNodeIDCounter,
			parentId='',						# Parent node ID
			displayName='',						# Customer display name
			address='',							# Customer address
			customerName='',					# Customer name
			download=1000,						# Customer download
			upload=1000							# Customer upload
		)
		net.addRawNode(customer)
		
		device = NetworkNode(
			id=100000 + parentNodeIDCounter,
			displayName='', 					# Device display name
			type=NodeType.device,
			parentId=parentNodeIDCounter,
			mac=serviceItem['mac'],
			ipv4=your_devices_ipv4_list,		# Device IPv4 list
			ipv6=your_devices_ipv6_list			# Device IPv6 list
		)
		net.addRawNode(device)
		parentNodeIDCounter = parentNodeIDCounter + 1

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

def importFromCustom():
	"""
	Entry point for the script to initiate the Custom data import and shaper creation process.
	"""
	createShaper()

if __name__ == '__main__':
	importFromCustom()
