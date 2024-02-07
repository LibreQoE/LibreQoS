from pythonCheck import checkPythonVersion
checkPythonVersion()
import os
import csv
import json
from liblqos_python import overwrite_network_json_always
from integrationCommon import NetworkGraph, NetworkNode, NodeType

def csvToNetworkJSONfile():
	sites = []
	with open('manualNetwork.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		for row in csv_reader:
			if 'Site Name' in row[0]:
				header = row
			else:
				name, down, up, parent = row
				site = {'name': name,
				'download': down,
				'upload': up,
				'parent': parent}
				sites.append(site)
	
	net = NetworkGraph()
	idCounter = 1000
	nameToID = {}
	for site in sites:
		site['id'] = idCounter
		idCounter = idCounter + 1
		nameToID[site['name']] = site['id']
	for site in sites:
		id = site['id']
		if site['parent'] == '':
			parentID = None
		else:
			parentID = nameToID[site['parent']]
		name = site['name']
		parent = site['parent']
		download = site['download']
		upload = site['upload']
		nodeType = NodeType.site
		node = NetworkNode(id=id, displayName=name, type=nodeType,
						   parentId=parentID, download=download, upload=upload, address=None, customerName=None)
		net.addRawNode(node)
	net.prepareTree()
	net.plotNetworkGraph(False)
	if net.doesNetworkJsonExist():
		if overwrite_network_json_always():
			net.createNetworkJson()
		else:
			print("network.json already exists and overwriteNetworkJSONalways set to False. Leaving in-place.")
	else:
		net.createNetworkJson()
	net.createShapedDevices()

if __name__ == '__main__':
	csvToNetworkJSONfile()
