from pythonCheck import checkPythonVersion
checkPythonVersion()

import requests
import warnings
import os
import csv
from liblqos_python import exclude_sites, find_ipv6_using_mikrotik, bandwidth_overhead_factor, overwrite_network_json_always, wispgate_api_token, wispgate_api_url

from integrationCommon import isIpv4Permitted
if find_ipv6_using_mikrotik() == True:
	from mikrotikFindIPv6 import pullMikrotikIPv6  
from integrationCommon import NetworkGraph, NetworkNode, NodeType
import os
import csv

def wispgate_request(target):
	data = {
		'api_token': wispgate_api_token(),
	}
	url = wispgate_api_url() + "/api/libreqos/" + target
	r = requests.post(url, data=data, timeout=120)
	return r.json()

def buildSiteBandwidths():
	"""
	Build a dictionary of site bandwidths by reading data from a CSV file.
	"""
	siteBandwidth = {}
	if os.path.isfile("integrationWISPGateBandwidths.csv"):
		with open('integrationWISPGateBandwidths.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				name, download, upload = row
				download = int(float(download))
				upload = int(float(upload))
				siteBandwidth[name] = {"download": download, "upload": upload}
	return siteBandwidth

def getShapedDevices():
	"""
	Retrieve all customer data from WISPGate API.
	"""
	return wispgate_request("shaped-devices")

def createShaper():
	"""
	Main function to fetch data from WISPGate, build the network graph, and shape devices.
	"""
	net = NetworkGraph()

	print("Fetching Shaped Devices from WISPGate")
	shaped_devices = getShapedDevices()
	print("Successfully fetched data from WISPGate")
	
	circuit_id_info = {}
	for device in shaped_devices:
		if device['Circuit ID'] not in circuit_id_info:
			try:
				download_max = 2
				upload_max = 2
				if device['Download Max Mbps'] != None:
					download_max = int(device['Download Max Mbps'])
				else:
					if device['Download Min Mbps'] != None:
						download_max = int(device['Download Min Mbps'])
				if device['Upload Max Mbps'] != None:
					upload_max = int(device['Upload Max Mbps'])
				else:
					if device['Upload Min Mbps'] != None:
						download_max = int(device['Upload Min Mbps'])
				entry = {
									'Circuit Name': device['Circuit ID'],
									'Parent Node': device['Parent Node'],
									'Download Min Mbps': device['Download Min Mbps'],
									'Upload Min Mbps': device['Upload Min Mbps'],
									'download_max': download_max,
									'upload_max': upload_max, 
				}
				circuit_id_info[device['Circuit ID']] = entry
			except ValueError:
				print("Circuit ID " + str(device['Circuit ID']) + " has invalid bandwidth.")
	
	devices_by_circuit_id = {}
	for device in shaped_devices:
		if device['Circuit ID'] not in devices_by_circuit_id:
			devices_by_circuit_id[device['Circuit ID']] = []
		temp = devices_by_circuit_id[device['Circuit ID']]
		has_at_least_one_ip = False
		if device['IPv4'] != None:
			if device['IPv4'] != '':
				has_at_least_one_ip = True
		if device['IPv6'] != None:
			if device['IPv6'] != '':
				has_at_least_one_ip = True
		if has_at_least_one_ip:
			temp.append(device)
		else:
			print("Omitted device " + device['Device Name'] + " from import due to lack of IP address.")
		devices_by_circuit_id[device['Circuit ID']] = temp
	
	device_id_counter = 10000
	circuits_added_counter = 0
	for circuit_id in circuit_id_info:
		# Only add circuits with one or more valid devices
		if len(devices_by_circuit_id[circuit_id]) > 0:
			customer = NetworkNode(
				type=NodeType.client,
				id=circuit_id,
				parentId='',
				displayName=circuit_id_info[circuit_id]['Circuit Name'],
				address=circuit_id_info[circuit_id]['Circuit Name'],
				customerName=circuit_id_info[circuit_id]['Circuit Name'],
				download=circuit_id_info[circuit_id]['download_max'],
				upload=circuit_id_info[circuit_id]['upload_max']
			)
			net.addRawNode(customer)
			for device in devices_by_circuit_id[circuit_id]:
				ipv4 = []
				ipv6 = []
				if device['IPv4'] != None:
					ipv4 = [device['IPv4']]
				if device['IPv6'] != None:
					ipv6 = [device['IPv6']]
				device = NetworkNode(
					id=device_id_counter,
					displayName=device['Device Name'],
					type=NodeType.device,
					parentId=circuit_id,
					mac=device['MAC'],
					ipv4=ipv4,
					ipv6=ipv6
				)
				net.addRawNode(device)
				device_id_counter += 1
				circuits_added_counter += 1
	print("Imported " + "{:.0%}".format(circuits_added_counter/len(shaped_devices)) + " of known shaped devices from WISPGate.")
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

def importFromWISPGate():
	"""
	Entry point for the script to initiate the WISPGate data import and shaper creation process.
	"""
	createShaper()

if __name__ == '__main__':
	importFromWISPGate()
