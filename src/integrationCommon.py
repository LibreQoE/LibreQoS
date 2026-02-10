# Optimized version of integrationCommon.py for better performance with large datasets
# Provides common functionality shared between integrations.

from typing import List, Any, Dict, Set
from liblqos_python import allowed_subnets, ignore_subnets, generated_pn_download_mbps, generated_pn_upload_mbps, \
	circuit_name_use_address, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
	find_ipv6_using_mikrotik, exclude_sites, bandwidth_overhead_factor, committed_bandwidth_multiplier, \
	exception_cpes, promote_to_root_list, client_bandwidth_multiplier
import ipaddress
import enum
import json
import os

def isInAllowedSubnets(inputIP):
	# Check whether an IP address occurs inside the allowedSubnets list
	isAllowed = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in allowed_subnets():
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isAllowed = True
	return isAllowed


def isInIgnoredSubnets(inputIP):
	# Check whether an IP address occurs within the ignoreSubnets list
	isIgnored = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in ignore_subnets():
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isIgnored = True
	return isIgnored


def isIpv4Permitted(inputIP):
	# Checks whether an IP address is in Allowed Subnets.
	# If it is, check that it isn't in Ignored Subnets.
	# If it is allowed and not ignored, returns true.
	# Otherwise, returns false.
	return isInIgnoredSubnets(inputIP) == False and isInAllowedSubnets(inputIP)


def fixSubnet(inputIP):
	# If an IP address has a CIDR other than /32 (e.g. 192.168.1.1/24),
	# but doesn't appear as a network address (e.g. 192.168.1.0/24)
	# then it probably isn't actually serving that whole subnet.
	# This allows you to specify e.g. 192.168.1.0/24 is "the client
	# on port 3" in the device, without falling afoul of UISP's inclusion
	# of subnet masks in device IPs.
	[rawIp, cidr] = inputIP.split('/')
	if cidr != "32":
		try:
			subnet = ipaddress.ip_network(inputIP)
		except:
			# Not a network address
			return rawIp + "/32"
	return inputIP

class NodeType(enum.IntEnum):
	# Enumeration to define what type of node
	# a NetworkNode is.
	root = 1
	site = 2
	ap = 3
	client = 4
	clientWithChildren = 5
	device = 6

def nodeTypeToString(integer):
	string = ''
	match integer:
		case 1: 
			string = 'root'
		case 2: 
			string = 'site'
		case 3: 
			string = 'ap'
		case 4: 
			string = 'client'
		case 5: 
			string = 'clientWithChildren'
		case 6: 
			string = 'device'
	return(string)

class NetworkNode:
	# Defines a node on a LibreQoS network graph.
	# Nodes default to being disconnected, and
	# will be mapped to the root of the overall
	# graph.

	id: str
	displayName: str
	parentIndex: int
	parentId: str
	type: NodeType
	downloadMbps: int
	uploadMbps: int
	ipv4: List
	ipv6: List
	address: str
	mac: str

	def __init__(self, id: str, displayName: str = "", parentId: str = "", type: NodeType = NodeType.site, download: int = generated_pn_download_mbps(), upload: int = generated_pn_upload_mbps(), ipv4: List = [], ipv6: List = [], address: str = "", mac: str = "", customerName: str = "") -> None:
		self.id = id
		self.parentIndex = 0
		self.type = type
		self.parentId = parentId
		if displayName == "":
			self.displayName = id
		else:
			self.displayName = displayName
		self.downloadMbps = download
		self.uploadMbps = upload
		self.ipv4 = ipv4
		self.ipv6 = ipv6
		self.address = address
		self.customerName = customerName
		self.mac = mac


class NetworkGraph:
	# OPTIMIZED VERSION: Uses dictionaries and caching for O(1) lookups
	# instead of O(n) linear searches
	
	nodes: List
	ipv4ToIPv6: Any
	excludeSites: List # Copied to allow easy in-test patching
	exceptionCPEs: Any
	
	# Performance optimization: Index structures for O(1) lookups
	_id_to_index: Dict[str, int]  # Maps node ID to index in nodes list
	_name_to_index: Dict[str, int]  # Maps display name to index
	_children_cache: Dict[int, List[int]]  # Cache parent->children mapping
	_cache_valid: bool  # Flag to invalidate cache when nodes change

	def __init__(self) -> None:
		self.nodes = [
			NetworkNode("FakeRoot", type=NodeType.root,
						parentId="", displayName="Shaper Root")
		]
		self.excludeSites = exclude_sites()
		self.exceptionCPEs = exception_cpes()
		self.errors: List[str] = []
		
		# Initialize optimization structures
		self._id_to_index = {"FakeRoot": 0}
		self._name_to_index = {"Shaper Root": 0}
		self._children_cache = {}
		self._cache_valid = False
		
		if find_ipv6_using_mikrotik():
			csv_path = "mikrotikDHCPRouterList.csv"
			try:
				from mikrotikFindIPv6 import pullMikrotikIPv6  
				mikrotik_map = pullMikrotikIPv6(csv_path)
				if isinstance(mikrotik_map, str):
					mikrotik_map = json.loads(mikrotik_map)
				self.ipv4ToIPv6 = mikrotik_map
			except FileNotFoundError:
				self.errors.append("Mikrotik IPv6 enrichment skipped: missing mikrotikDHCPRouterList.csv")
				self.ipv4ToIPv6 = {}
			except json.JSONDecodeError as exc:
				self.errors.append(f"Mikrotik IPv6 enrichment skipped: unable to parse Mikrotik data ({exc})")
				self.ipv4ToIPv6 = {}
			except ModuleNotFoundError as exc:
				self.errors.append(f"Mikrotik IPv6 enrichment skipped: missing dependency ({exc})")
				self.ipv4ToIPv6 = {}
			except Exception as exc:
				self.errors.append(f"Mikrotik IPv6 enrichment failed: {exc}")
				self.ipv4ToIPv6 = {}
		else:
			self.ipv4ToIPv6 = {}

	def addRawNode(self, node: NetworkNode) -> None:
		# Adds a NetworkNode to the graph, unchanged.
		# If a site is excluded (via excludedSites in lqos.conf)
		# it won't be added
		if not node.displayName in self.excludeSites:
			# Add to main list
			idx = len(self.nodes)
			self.nodes.append(node)
			
			# Update indexes for O(1) lookup
			self._id_to_index[node.id] = idx
			self._name_to_index[node.displayName] = idx
			
			# Invalidate cache
			self._cache_valid = False

	def getErrors(self) -> List[str]:
		return list(self.errors)

	def replaceRootNode(self, node: NetworkNode) -> None:
		# Replaces the automatically generated root node
		# with a new node. Useful when you have a top-level
		# node specified (e.g. "uispSite" in the UISP
		# integration)
		old_node = self.nodes[0]
		self.nodes[0] = node
		
		# Update indexes
		if old_node.id in self._id_to_index:
			del self._id_to_index[old_node.id]
		if old_node.displayName in self._name_to_index:
			del self._name_to_index[old_node.displayName]
		
		self._id_to_index[node.id] = 0
		self._name_to_index[node.displayName] = 0
		self._cache_valid = False

	def addNodeAsChild(self, parent: str, node: NetworkNode) -> None:
		# Searches the existing graph for a named parent,
		# adjusts the new node's parentIndex to match the new
		# node. The parented node is then inserted.
		#
		# Exceptions are NOT applied, since we're explicitly
		# specifying the parent - we're assuming you really
		# meant it.
		if node.displayName in self.excludeSites: return
		
		# O(1) lookup instead of O(n) search
		parentIdx = self._id_to_index.get(parent, 0)
		node.parentIndex = parentIdx
		
		idx = len(self.nodes)
		self.nodes.append(node)
		
		# Update indexes
		self._id_to_index[node.id] = idx
		self._name_to_index[node.displayName] = idx
		self._cache_valid = False

	def __reparentById(self) -> None:
		# OPTIMIZED: Uses dictionary lookups instead of nested O(n²) loops
		cached_root_list = promote_to_root_list()
		
		# Build a set for O(1) membership testing
		root_list_set = set(cached_root_list)
		
		for child in self.nodes:
			if child.parentId != "":
				# O(1) lookup instead of O(n) search
				parent_idx = self._id_to_index.get(child.parentId, -1)
				
				if parent_idx != -1:
					# Check if parent should be promoted to root
					parent_node = self.nodes[parent_idx]
					if parent_node.displayName in root_list_set:
						child.parentIndex = 0
					else:
						child.parentIndex = parent_idx
		
		# Invalidate children cache since parent relationships changed
		self._cache_valid = False

	def findNodeIndexById(self, id: str) -> int:
		# OPTIMIZED: O(1) dictionary lookup instead of O(n) search
		return self._id_to_index.get(id, -1)

	def findNodeIndexByName(self, name: str) -> int:
		# OPTIMIZED: O(1) dictionary lookup instead of O(n) search
		return self._name_to_index.get(name, -1)

	def _buildChildrenCache(self) -> None:
		# Build parent->children mapping cache
		self._children_cache = {}
		for i, node in enumerate(self.nodes):
			parent_idx = node.parentIndex
			if parent_idx not in self._children_cache:
				self._children_cache[parent_idx] = []
			self._children_cache[parent_idx].append(i)
		self._cache_valid = True

	def findChildIndices(self, parentIndex: int) -> List:
		# OPTIMIZED: O(1) cached lookup instead of O(n) search
		if not self._cache_valid:
			self._buildChildrenCache()
		return self._children_cache.get(parentIndex, [])

	def __promoteClientsWithChildren(self) -> None:
		# OPTIMIZED: Single pass with cached children lookup
		if not self._cache_valid:
			self._buildChildrenCache()
		
		for i, node in enumerate(self.nodes):
			if node.type == NodeType.client:
				# O(1) lookup of children
				children = self._children_cache.get(i, [])
				for child_idx in children:
					if self.nodes[child_idx].type != NodeType.device:
						node.type = NodeType.clientWithChildren
						break  # No need to check other children

	def __clientsWithChildrenToSites(self) -> None:
		# OPTIMIZED: Batch processing with deferred reparenting
		toAdd = []
		reparent_map = {}  # Store reparenting operations
		
		if not self._cache_valid:
			self._buildChildrenCache()
		
			for i, node in enumerate(self.nodes):
				if node.type == NodeType.clientWithChildren:
					siteNode = NetworkNode(
						id=str(node.id) + "_gen",
						displayName="(Generated Site) " + node.displayName,
						type=NodeType.site
					)
					siteNode.parentIndex = node.parentIndex
					node.parentId = siteNode.id
					node.type = NodeType.client
					
					# Store reparenting operations for batch processing
					children = self._children_cache.get(i, [])
					for child_idx in children:
						child = self.nodes[child_idx]
						if child.type in (NodeType.client, NodeType.clientWithChildren, NodeType.site):
							reparent_map[child_idx] = siteNode.id
					
					toAdd.append(siteNode)
		
		# Batch add new nodes
		for n in toAdd:
			self.addRawNode(n)
		
		# Batch reparent
		for child_idx, new_parent_id in reparent_map.items():
			self.nodes[child_idx].parentId = new_parent_id
		
		self.__reparentById()

	def __findUnconnectedNodes(self) -> List:
		# OPTIMIZED: Uses set for O(1) membership testing
		visited = set()
		next = [0]
		
		if not self._cache_valid:
			self._buildChildrenCache()
		
		while next:
			current = next.pop()
			if current in visited:
				continue
			visited.add(current)
			
			# O(1) lookup of children
			children = self._children_cache.get(current, [])
			for child_idx in children:
				if child_idx not in visited:
					next.append(child_idx)
		
		# Find unvisited nodes
		all_indices = set(range(len(self.nodes)))
		unconnected = list(all_indices - visited)
		return unconnected

	def __reconnectUnconnected(self):
		# OPTIMIZED: Single pass with type grouping
		unconnected = self.__findUnconnectedNodes()
		
		# Group by type for batch processing
		by_type = {
			NodeType.site: [],
			NodeType.clientWithChildren: [],
			NodeType.client: []
		}
		
		for idx in unconnected:
			node_type = self.nodes[idx].type
			if node_type in by_type:
				by_type[node_type].append(idx)
		
		# Reconnect in priority order
		for node_type in [NodeType.site, NodeType.clientWithChildren, NodeType.client]:
			for idx in by_type[node_type]:
				self.nodes[idx].parentIndex = 0
		
		self._cache_valid = False

	def prepareTree(self) -> None:
		# Helper function that calls all the cleanup and mapping
		# functions in the right order. Unless you are doing
		# something special, you can use this instead of
		# calling the functions individually
		print("PrepareTree: Starting reparenting...")
		self.__reparentById()
		print("PrepareTree: Promoting clients with children...")
		self.__promoteClientsWithChildren()
		print("PrepareTree: Converting clients with children to sites...")
		self.__clientsWithChildrenToSites()
		print("PrepareTree: Reconnecting unconnected nodes...")
		self.__reconnectUnconnected()
		print("PrepareTree: Complete")

	def doesNetworkJsonExist(self):
		# Returns true if "network.json" exists, false otherwise
		import os
		return os.path.isfile("network.json")

	def __isSite(self, index) -> bool:
		return self.nodes[index].type == NodeType.ap or self.nodes[index].type == NodeType.site or self.nodes[index].type == NodeType.clientWithChildren

	def createNetworkJson(self):
		import json
		topLevelNode = {}
		self.__visited = []  # Protection against loops - never visit twice

		if not self._cache_valid:
			self._buildChildrenCache()

		# O(1) lookup of root's children
		root_children = self._children_cache.get(0, [])
		for child in root_children:
			if child > 0 and self.__isSite(child):
				topLevelNode[self.nodes[child].displayName] = self.__buildNetworkObject(child)

		del self.__visited
		
		def inheritBandwidthMaxes(data, parentMaxDL, parentMaxUL):
			for node in data:
				if isinstance(node, str):
					if (isinstance(data[node], dict)) and (node != 'children'):
						data[node]['downloadBandwidthMbps'] = min(int(data[node]['downloadBandwidthMbps']),int(parentMaxDL))
						data[node]['uploadBandwidthMbps'] = min(int(data[node]['uploadBandwidthMbps']),int(parentMaxUL))
						if 'children' in data[node]:
							inheritBandwidthMaxes(data[node]['children'], data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'])
		inheritBandwidthMaxes(topLevelNode, parentMaxDL=upstream_bandwidth_capacity_download_mbps(), parentMaxUL=upstream_bandwidth_capacity_upload_mbps())
		
		with open('network.json', 'w') as f:
			json.dump(topLevelNode, f, indent=4)

	def __buildNetworkObject(self, idx):
		# Private: used to recurse down the network tree while building
		# network.json
		self.__visited.append(idx)
		node = {
			"downloadBandwidthMbps": self.nodes[idx].downloadMbps,
			"uploadBandwidthMbps": self.nodes[idx].uploadMbps,
			'type': nodeTypeToString(self.nodes[idx].type),
		}
		children = {}
		hasChildren = False
		
		# O(1) lookup of children
		child_indices = self._children_cache.get(idx, [])
		for child in child_indices:
			if child > 0 and self.__isSite(child) and child not in self.__visited:
				children[self.nodes[child].displayName] = self.__buildNetworkObject(child)
				hasChildren = True
		
		if hasChildren:
			node["children"] = children
		return node

	def __addIpv6FromMap(self, ipv4, ipv6) -> None:
		# Scans each address in ipv4. If its present in the
		# IPv4 to Ipv6 map (currently pulled from Mikrotik devices
		# if findIPv6usingMikrotik is enabled), then matching
		# IPv6 networks are appended to the ipv6 list.
		# This is explicitly non-destructive of the existing IPv6
		# list, in case you already have some.
		for ipCidr in ipv4:
			if '/' in ipCidr: ip = ipCidr.split('/')[0]
			else: ip = ipCidr
			if ip in self.ipv4ToIPv6.keys():
				ipv6.append(self.ipv4ToIPv6[ip])

	def createShapedDevices(self):
		import csv
		# Builds ShapedDevices.csv from the network tree.
		circuits = []
		
		if not self._cache_valid:
			self._buildChildrenCache()
		
		for i, node in enumerate(self.nodes):
			if node.type == NodeType.client:
				parent = self.nodes[node.parentIndex].displayName
				if parent == "Shaper Root": parent = ""
				
				if circuit_name_use_address():
					displayNameToUse = node.address
				else:
					if node.type == NodeType.client:
						displayNameToUse = node.displayName
					else:
						displayNameToUse = node.customerName + " (" + nodeTypeToString(node.type) + ")"
				
				circuit = {
					"id": node.id,
					"name": displayNameToUse,
					"parent": parent,
					"download": node.downloadMbps,
					"upload": node.uploadMbps,
					"devices": []
				}
				
				# O(1) lookup of children
				child_indices = self._children_cache.get(i, [])
				for child_idx in child_indices:
					child = self.nodes[child_idx]
					if child.type == NodeType.device and (len(child.ipv4) + len(child.ipv6) > 0):
						ipv4 = child.ipv4
						ipv6 = child.ipv6
						self.__addIpv6FromMap(ipv4, ipv6)
						device = {
							"id": child.id,
							"name": child.displayName,
							"mac": child.mac,
							"ipv4": ipv4,
							"ipv6": ipv6,
						}
						circuit["devices"].append(device)
				
				if len(circuit["devices"]) > 0:
					circuits.append(circuit)
		
		with open('ShapedDevices.csv', 'w', newline='') as csvfile:
			wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
			wr.writerow(['Circuit ID', 'Circuit Name', 'Device ID', 'Device Name', 'Parent Node', 'MAC',
						 'IPv4', 'IPv6', 'Download Min', 'Upload Min', 'Download Max', 'Upload Max', 'Comment'])
			for circuit in circuits:
				for device in circuit["devices"]:
					#Remove brackets and quotes of list so LibreQoS.py can parse it
					device["ipv4"] = str(device["ipv4"]).replace('[','').replace(']','').replace("'",'')
					device["ipv6"] = str(device["ipv6"]).replace('[','').replace(']','').replace("'",'')
					if circuit["upload"] is None: 
						circuit["upload"] = 0.0
					if circuit["download"] is None: 
						circuit["download"] = 0.0
					row = [
						circuit["id"],
						circuit["name"],
						device["id"],
						device["name"],
						circuit["parent"],
						device["mac"],
						device["ipv4"],
						device["ipv6"],
						int(1),
						int(1),
						max(1.0, round(float(circuit["download"]), 2)),
						max(1.0, round(float(circuit["upload"]), 2)),
						""
					]
					wr.writerow(row)
			
			# If we have an "appendToShapedDevices.csv" file, it gets appended to the end of the file.
			# This is useful for adding devices that are not in the network tree, such as a
			# "default" device that gets all the traffic that doesn't match any other device.
			if os.path.isfile('appendToShapedDevices.csv'):
				with open('appendToShapedDevices.csv', 'r') as f:
					reader = csv.reader(f)
					for row in reader:
						wr.writerow(row)

	def plotNetworkGraph(self, showClients=False):
		# Requires `pip install graphviz` to function.
		# You also need to install graphviz on your PC.
		# In Ubuntu, apt install graphviz will do it.
		# Plots the network graph to a PDF file, allowing
		# visual verification that the graph makes sense.
		# Could potentially be useful in a future
		# web interface.
		import importlib.util
		if (spec := importlib.util.find_spec('graphviz')) is None:
			return

		import graphviz
		dot = graphviz.Digraph(
			'network', comment="Network Graph", engine="dot", graph_attr={'rankdir':'LR'})

		if not self._cache_valid:
			self._buildChildrenCache()

		for i, node in enumerate(self.nodes):
			if ((node.type != NodeType.client and node.type != NodeType.device) or showClients):
				color = "white"
				match node.type:
					case NodeType.root: color = "green"
					case NodeType.site: color = "red"
					case NodeType.ap: color = "blue"
					case NodeType.clientWithChildren: color = "magenta"
					case NodeType.device: color = "white"
					case default: color = "grey"
				dot.node("N" + str(i), node.displayName, color=color)
				
				# O(1) lookup of children
				children = self._children_cache.get(i, [])
				for child in children:
					if child != i:
						if (self.nodes[child].type != NodeType.client and self.nodes[child].type != NodeType.device) or showClients:
							dot.edge("N" + str(i), "N" + str(child))
		dot = dot.unflatten(stagger=3)#, fanout=True)
		dot.render("network")
