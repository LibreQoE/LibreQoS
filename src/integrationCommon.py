# Optimized version of integrationCommon.py for better performance with large datasets
# Provides common functionality shared between integrations.

from typing import List, Any, Dict, Set
from liblqos_python import allowed_subnets, ignore_subnets, generated_pn_download_mbps, generated_pn_upload_mbps, \
	circuit_name_use_address, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
	find_ipv6_using_mikrotik, exclude_sites, bandwidth_overhead_factor, committed_bandwidth_multiplier, \
	exception_cpes, promote_to_root_list, client_bandwidth_multiplier, \
	write_compiled_topology_from_python_graph_payload
import ipaddress
import enum
import json
import os
import time

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


def isIpPermitted(inputIP):
	# Generic form of subnet permission checking used by integrations.
	# The configured allow/ignore ranges may include either IPv4 or IPv6.
	return isIpv4Permitted(inputIP)


def isIntegrationOutputIpAllowed(inputIP):
	# Shared integration-output pruning is intentionally narrower than
	# full "permitted" checks: only explicitly ignored subnets are removed.
	return not isInIgnoredSubnets(inputIP)


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


def apply_client_bandwidth_multiplier(plan_rate_mbps):
	# Convert a raw client/service plan rate into the effective shaped rate.
	# The higher of bandwidth_overhead_factor and client_bandwidth_multiplier wins.
	plan_rate_mbps = float(plan_rate_mbps or 0.0)
	if plan_rate_mbps <= 0:
		return 0.0
	overhead = bandwidth_overhead_factor()
	minimum = client_bandwidth_multiplier()
	adjusted = plan_rate_mbps * overhead
	floor_value = plan_rate_mbps * minimum
	return max(adjusted, floor_value)

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

def syntheticNetworkJsonId(scope: str, nodeType: str, name: str) -> str:
	parts = []
	for ch in str(name).lower():
		if ch.isalnum():
			parts.append(ch)
		elif not parts or parts[-1] != '-':
			parts.append('-')
	slug = ''.join(parts).strip('-')
	return f"libreqos:generated:{scope}:{nodeType}:{slug}"

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
	networkJsonId: str
	latitude: float
	longitude: float

	def __init__(self, id: str, displayName: str = "", parentId: str = "", type: NodeType = NodeType.site, download: int = generated_pn_download_mbps(), upload: int = generated_pn_upload_mbps(), ipv4: List = [], ipv6: List = [], address: str = "", mac: str = "", customerName: str = "", networkJsonId: str = "", latitude = None, longitude = None) -> None:
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
		self.networkJsonId = networkJsonId
		self.latitude = latitude
		self.longitude = longitude


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
		node.parentId = parent
		
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
			child.parentIndex = 0
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
				original_parent_id = node.parentId
				siteNode = NetworkNode(
					id=str(node.id) + "_gen",
					displayName="(Generated Site) " + node.displayName,
					type=NodeType.site,
					networkJsonId=syntheticNetworkJsonId("graph", "site", node.displayName),
				)
				siteNode.parentIndex = node.parentIndex
				siteNode.parentId = original_parent_id
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

	def __liftTopologyChildrenOutOfClients(self) -> None:
		# Keep exportable topology nodes as siblings of a circuit's generated site,
		# never as children of the circuit itself. Otherwise nested generated sites
		# become unreachable from network.json because client nodes are not exported.
		if not self._cache_valid:
			self._buildChildrenCache()

		reparent_map = {}
		for i, node in enumerate(self.nodes):
			if node.type != NodeType.client:
				continue
			new_parent_id = node.parentId
			if new_parent_id in (None, ""):
				continue
			for child_idx in self._children_cache.get(i, []):
				child = self.nodes[child_idx]
				if child.type in (NodeType.client, NodeType.clientWithChildren, NodeType.site):
					reparent_map[child_idx] = new_parent_id

		if not reparent_map:
			return

		for child_idx, new_parent_id in reparent_map.items():
			self.nodes[child_idx].parentId = new_parent_id

		self.__reparentById()

	def __findUnconnectedNodes(self) -> List:
		# Rebuild the cache each time because callers may have adjusted
		# parentIndex directly as part of repair/testing flows.
		visited = set()
		next = [0]
		
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
		print("PrepareTree: Lifting topology descendants out of client nodes...")
		self.__liftTopologyChildrenOutOfClients()
		print("PrepareTree: Reconnecting unconnected nodes...")
		self.__reconnectUnconnected()
		print("PrepareTree: Pruning ignored-only devices and empty circuits...")
		self.__pruneIgnoredCircuits()
		print("PrepareTree: Complete")

	def doesNetworkJsonExist(self):
		# Returns true if "network.json" exists, false otherwise
		import os
		return os.path.isfile("network.json")

	def __isSite(self, index) -> bool:
		return self.nodes[index].type == NodeType.ap or self.nodes[index].type == NodeType.site or self.nodes[index].type == NodeType.clientWithChildren

	def createNetworkJson(self):
		import json
		topLevelNode = self.buildNetworkJson()
		with open('network.json', 'w') as f:
			json.dump(topLevelNode, f, indent=4)

	def buildNetworkJson(self):
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
		return topLevelNode

	def __buildNetworkObject(self, idx):
		# Private: used to recurse down the network tree while building
		# network.json
		self.__visited.append(idx)
		node = {
			"downloadBandwidthMbps": self.nodes[idx].downloadMbps,
			"uploadBandwidthMbps": self.nodes[idx].uploadMbps,
			'type': nodeTypeToString(self.nodes[idx].type),
		}
		if self.nodes[idx].networkJsonId:
			node["id"] = self.nodes[idx].networkJsonId
		if self.nodes[idx].latitude is not None and self.nodes[idx].longitude is not None:
			node["latitude"] = self.nodes[idx].latitude
			node["longitude"] = self.nodes[idx].longitude
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

	def __rebuildIndexes(self) -> None:
		self._id_to_index = {}
		self._name_to_index = {}
		for idx, node in enumerate(self.nodes):
			self._id_to_index[node.id] = idx
			self._name_to_index[node.displayName] = idx
		self._cache_valid = False

	def __filterPermittedIps(self, ip_list: List[str]) -> List[str]:
		return [ip for ip in ip_list if isIntegrationOutputIpAllowed(ip)]

	def __pruneIgnoredCircuits(self) -> None:
		for node in self.nodes:
			if node.type == NodeType.device:
				node.ipv4 = self.__filterPermittedIps(node.ipv4)
				node.ipv6 = self.__filterPermittedIps(node.ipv6)

		if not self._cache_valid:
			self._buildChildrenCache()

		removable = set()
		for idx, node in enumerate(self.nodes):
			if node.type == NodeType.device and (len(node.ipv4) + len(node.ipv6) == 0):
				removable.add(idx)

		for idx, node in enumerate(self.nodes):
			if node.type != NodeType.client:
				continue
			has_shapable_device = False
			for child_idx in self._children_cache.get(idx, []):
				if child_idx in removable:
					continue
				child = self.nodes[child_idx]
				if child.type == NodeType.device and (len(child.ipv4) + len(child.ipv6) > 0):
					has_shapable_device = True
					break
			if not has_shapable_device:
				removable.add(idx)

		if not removable:
			return

		self.nodes = [
			node for idx, node in enumerate(self.nodes)
			if idx not in removable
		]
		self.__rebuildIndexes()
		self.__reparentById()

	def createShapedDevices(self):
		shaped_devices_csv, circuit_anchor_file = self.buildShapedDevicesArtifacts()
		with open('ShapedDevices.csv', 'w', newline='') as csvfile:
			csvfile.write(shaped_devices_csv)

		with open('circuit_anchors.json', 'w', encoding='utf-8') as anchorfile:
			json.dump(circuit_anchor_file, anchorfile, indent=2)
			anchorfile.write('\n')

	def buildShapedDevicesArtifacts(self):
		import csv
		import io
		circuits = []
		circuit_anchors = []
		
		if not self._cache_valid:
			self._buildChildrenCache()

		def nearestRealTopologyParent(idx):
			parent_idx = self.nodes[idx].parentIndex
			while parent_idx > 0:
				parent_node = self.nodes[parent_idx]
				parent_id = parent_node.networkJsonId or ""
				if parent_id and not parent_id.startswith("libreqos:generated:graph:"):
					return parent_node
				parent_idx = parent_node.parentIndex
			return None
		
		for i, node in enumerate(self.nodes):
			if node.type == NodeType.client:
				parent_node = nearestRealTopologyParent(i)
				parent = parent_node.displayName if parent_node is not None else ""
				parent_id = parent_node.networkJsonId if parent_node is not None else ""
				if parent == "Shaper Root": parent = ""
				if parent == "":
					parent_id = ""
				
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
					"parent_id": parent_id,
					"download": node.downloadMbps,
					"upload": node.uploadMbps,
					"devices": []
				}
				anchor_id = node.networkJsonId
				if anchor_id:
					circuit_anchors.append({
						"circuit_id": node.id,
						"circuit_name": displayNameToUse,
						"anchor_node_id": anchor_id,
						"anchor_node_name": node.displayName if node.displayName else None,
					})
				
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

		csv_buffer = io.StringIO()
		wr = csv.writer(csv_buffer, quoting=csv.QUOTE_ALL)
		wr.writerow(['Circuit ID', 'Circuit Name', 'Device ID', 'Device Name', 'Parent Node', 'Parent Node ID', 'Anchor Node ID', 'MAC',
					 'IPv4', 'IPv6', 'Download Min', 'Upload Min', 'Download Max', 'Upload Max', 'Comment'])
		for circuit in circuits:
			for device in circuit["devices"]:
				ipv4 = str(device["ipv4"]).replace('[','').replace(']','').replace("'",'')
				ipv6 = str(device["ipv6"]).replace('[','').replace(']','').replace("'",'')
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
					circuit.get("parent_id", ""),
					"",
					device["mac"],
					ipv4,
					ipv6,
					int(1),
					int(1),
					max(1.0, round(float(circuit["download"]), 2)),
					max(1.0, round(float(circuit["upload"]), 2)),
					""
				]
				wr.writerow(row)
		
		if os.path.isfile('appendToShapedDevices.csv'):
			with open('appendToShapedDevices.csv', 'r') as f:
				reader = csv.reader(f)
				for row in reader:
					wr.writerow(row)

		return (
			csv_buffer.getvalue(),
			{
				"schema_version": 1,
				"source": "python/integration_common",
				"generated_unix": int(time.time()),
				"anchors": circuit_anchors,
			},
		)

	def _isInfrastructureTopologyNode(self, idx):
		if idx <= 0:
			return False
		node = self.nodes[idx]
		if node.type not in (NodeType.site, NodeType.ap):
			return False
		if not node.networkJsonId:
			return False
		# Customer-derived generated graph sites are compatibility-only and
		# must never participate in native topology editing/runtime state.
		if node.networkJsonId.startswith("libreqos:generated:graph:"):
			return False
		return True

	def _buildInfrastructureTopologyContext(self):
		if not self._cache_valid:
			self._buildChildrenCache()

		exportable_indices = [
			idx for idx, _node in enumerate(self.nodes)
			if self._isInfrastructureTopologyNode(idx)
		]
		exportable_set = set(exportable_indices)

		def nearestInfrastructureParentIndex(idx):
			parent_idx = self.nodes[idx].parentIndex
			while parent_idx > 0:
				if parent_idx in exportable_set:
					return parent_idx
				parent_idx = self.nodes[parent_idx].parentIndex
			return None

		resolved_parent_by_idx = {}
		children_by_parent = {}
		for idx in exportable_indices:
			parent_idx = nearestInfrastructureParentIndex(idx)
			resolved_parent_by_idx[idx] = parent_idx
			if parent_idx is None:
				continue
			children_by_parent.setdefault(parent_idx, []).append(idx)

		descendant_cache = {}

		def topology_descendants(idx):
			if idx in descendant_cache:
				return descendant_cache[idx]
			descendants = set()
			for child_idx in children_by_parent.get(idx, []):
				descendants.add(child_idx)
				descendants.update(topology_descendants(child_idx))
			descendant_cache[idx] = descendants
			return descendants

		return {
			"exportable_indices": exportable_indices,
			"resolved_parent_by_idx": resolved_parent_by_idx,
			"children_by_parent": children_by_parent,
			"topology_descendants": topology_descendants,
		}

	def _autoAttachmentOption(self):
		return {
			"attachment_id": "auto",
			"attachment_name": "Auto",
			"attachment_kind": "auto",
			"attachment_role": "unknown",
			"rate_source": "unknown",
			"can_override_rate": False,
			"has_rate_override": False,
			"probe_enabled": False,
			"probeable": False,
			"health_status": "disabled",
			"effective_selected": False,
		}

	def _isGeneratedInfrastructureNode(self, idx):
		if idx <= 0:
			return False
		node_id = self.nodes[idx].networkJsonId or ""
		return node_id.startswith("libreqos:generated:")

	def _classifyInfrastructureEditPolicy(self, idx, context):
		parent_idx = context["resolved_parent_by_idx"][idx]
		node = self.nodes[idx]
		if parent_idx is None:
			return "root"
		if node.networkJsonId.startswith("libreqos:generated:"):
			return "fixed_parent"
		candidate_parent_indices = self._boundedAlternativeParentIndices(idx, context)
		if len(candidate_parent_indices) > 1:
			return "movable"
		return "fixed_parent"

	def _boundedAlternativeParentIndices(self, idx, context):
		parent_idx = context["resolved_parent_by_idx"][idx]
		if parent_idx is None:
			return []

		candidate_indices = []
		seen = set()

		def include(candidate_idx):
			if candidate_idx is None:
				return
			if candidate_idx == idx:
				return
			if candidate_idx in seen:
				return
			if not self._isInfrastructureTopologyNode(candidate_idx):
				return
			if self._isGeneratedInfrastructureNode(candidate_idx):
				return
			seen.add(candidate_idx)
			candidate_indices.append(candidate_idx)

		# Always keep the current resolved parent.
		include(parent_idx)

		# Conservative bounded-move policy for Python-backed integrations:
		# allow reparenting only within the current parent's immediate neighborhood.
		grandparent_idx = context["resolved_parent_by_idx"].get(parent_idx)
		if grandparent_idx is not None:
			for sibling_parent_idx in context["children_by_parent"].get(grandparent_idx, []):
				if sibling_parent_idx == idx:
					continue
				include(sibling_parent_idx)
		else:
			parent_node = self.nodes[parent_idx]
			for root_candidate_idx in context["exportable_indices"]:
				if root_candidate_idx == parent_idx:
					continue
				if context["resolved_parent_by_idx"][root_candidate_idx] is not None:
					continue
				root_candidate = self.nodes[root_candidate_idx]
				if root_candidate.type != parent_node.type:
					continue
				include(root_candidate_idx)

		return candidate_indices

	def buildNativeTopologyEditorState(self, source: str):
		# Build the native infrastructure topology consumed by Topology Manager.
		# This intentionally excludes generated customer graph nodes and preserves
		# the resolved infrastructure parent chain from the prepared NetworkGraph.
		context = self._buildInfrastructureTopologyContext()
		exportable_indices = context["exportable_indices"]
		resolved_parent_by_idx = context["resolved_parent_by_idx"]
		topology_descendants = context["topology_descendants"]

		nodes = []
		for idx in exportable_indices:
			node = self.nodes[idx]
			parent_idx = resolved_parent_by_idx[idx]
			parent_node = self.nodes[parent_idx] if parent_idx is not None else None
			edit_policy = self._classifyInfrastructureEditPolicy(idx, context)
			can_move = edit_policy == "movable"

			allowed_parents = []
			if can_move:
				for candidate_idx in self._boundedAlternativeParentIndices(idx, context):
					if candidate_idx in topology_descendants(idx) or candidate_idx == idx:
						continue
					candidate = self.nodes[candidate_idx]
					allowed_parents.append({
						"parent_node_id": candidate.networkJsonId,
						"parent_node_name": candidate.displayName,
						"attachment_options": [self._autoAttachmentOption()],
						"all_attachments_suppressed": False,
						"has_probe_unavailable_attachments": False,
					})
				allowed_parents.sort(key=lambda entry: (entry["parent_node_name"].lower(), entry["parent_node_id"]))

			nodes.append({
				"node_id": node.networkJsonId,
				"node_name": node.displayName,
				"current_parent_node_id": parent_node.networkJsonId if parent_node is not None else None,
				"current_parent_node_name": parent_node.displayName if parent_node is not None else None,
				"current_attachment_id": None,
				"current_attachment_name": None,
				"can_move": can_move,
				"allowed_parents": allowed_parents,
				"queue_visibility_policy": (
					"queue_hidden_promote_children"
					if parent_node is None
					else "queue_auto"
					if node.type == NodeType.site
					else "queue_visible"
				),
				"preferred_attachment_id": None,
				"preferred_attachment_name": None,
				"effective_attachment_id": None,
				"effective_attachment_name": None,
			})

		return {
			"schema_version": 1,
			"source": source,
			"generated_unix": int(time.time()),
			"nodes": nodes,
		}

	def buildTopologyParentCandidates(self):
		# Build a legacy-compatible parent-candidate snapshot for topology editing.
		# This gives Python-backed integrations the same basic "Start Move" capability
		# as richer native importers by enumerating legal non-descendant topology parents
		# for real infrastructure branches only.
		context = self._buildInfrastructureTopologyContext()
		exportable_indices = context["exportable_indices"]
		resolved_parent_by_idx = context["resolved_parent_by_idx"]
		topology_descendants = context["topology_descendants"]

		nodes = []
		for idx in exportable_indices:
			node = self.nodes[idx]
			parent_idx = resolved_parent_by_idx[idx]
			edit_policy = self._classifyInfrastructureEditPolicy(idx, context)
			if edit_policy != "movable":
				continue
			parent_node = self.nodes[parent_idx]
			parent_node_id = parent_node.networkJsonId
			parent_node_name = parent_node.displayName

			candidate_parents = []
			for candidate_idx in self._boundedAlternativeParentIndices(idx, context):
				if candidate_idx in topology_descendants(idx) or candidate_idx == idx:
					continue
				candidate = self.nodes[candidate_idx]
				candidate_parents.append({
					"node_id": candidate.networkJsonId,
					"node_name": candidate.displayName,
				})
			candidate_parents.sort(key=lambda entry: (entry["node_name"].lower(), entry["node_id"]))

			nodes.append({
				"node_id": node.networkJsonId,
				"node_name": node.displayName,
				"current_parent_node_id": parent_node_id,
				"current_parent_node_name": parent_node_name,
				"candidate_parents": candidate_parents,
			})

		return {
			"source": "python/integration_common",
			"nodes": nodes,
		}

	def materializeCompiledTopology(self, source: str, compileMode: str) -> None:
		# Materializes compiler-owned topology artifacts without writing integration
		# network.json or ShapedDevices.csv.
		compatibility_network_json = json.dumps(self.buildNetworkJson())
		shaped_devices_csv, circuit_anchor_file = self.buildShapedDevicesArtifacts()
		parent_candidates_json = json.dumps(self.buildTopologyParentCandidates())
		native_editor_json = json.dumps(self.buildNativeTopologyEditorState(source))
		write_compiled_topology_from_python_graph_payload(
			source,
			compileMode,
			compatibility_network_json,
			shaped_devices_csv,
			json.dumps(circuit_anchor_file),
			parent_candidates_json,
			native_editor_json,
		)

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
