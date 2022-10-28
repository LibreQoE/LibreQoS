# Provides common functionality shared between
# integrations.

from typing import List
from ispConfig import allowedSubnets, ignoreSubnets, generatedPNUploadMbps, generatedPNDownloadMbps
import ipaddress;
import enum

def isInAllowedSubnets(inputIP):
    # Check whether an IP address occurs inside the allowedSubnets list
	isAllowed = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in allowedSubnets:
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isAllowed = True
	return isAllowed

def isInIgnoredSubnets(inputIP):
    # Check whether an IP address occurs within the ignoreSubnets list
	isIgnored = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in ignoreSubnets:
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isIgnored = True
	return isIgnored

def isIpv4Permitted(inputIP):
    # Checks whether an IP address is in Allowed Subnets.
    # If it is, check that it isn't in Ignored Subnets.
    # If it is allowed and not ignored, returns true.
    # Otherwise, returns false.
    return isInIgnoredSubnets(inputIP)==False and isInAllowedSubnets(inputIP)

class NodeType(enum.IntEnum):
	# Enumeration to define what type of node
	# a NetworkNode is.
	root = 1
	site = 2
	ap = 3
	client = 4
	clientWithChildren = 5
	device = 6

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

	def __init__(self, id: str, displayName: str = "", parentId: str = "", type: NodeType = NodeType.site, download:int = generatedPNDownloadMbps, upload:int = generatedPNUploadMbps) -> None:
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

class NetworkGraph:
	# Defines a network as a graph topology
	# allowing any integration to build the
	# graph via a common API, emitting
	# ShapedDevices and network.json files
	# via a common interface.

	nodes: List

	def __init__(self) -> None:
		self.nodes = [
			NetworkNode("FakeRoot", type=NodeType.root, parentId="", displayName="Shaper Root")
		]

	def addRawNode(self, node: NetworkNode) -> None:
		# Adds a NetworkNode to the graph, unchanged.
		self.nodes.append(node)

	def replaceRootNote(self, node: NetworkNode) -> None:
		# Replaces the automatically generated root node
		# with a new node. Useful when you have a top-level
		# node specified (e.g. "uispSite" in the UISP
		# integration)
		self.nodes[0] = node

	def addNodeAsChild(self, parent: str, node: NetworkNode) -> None:
		# Searches the existing graph for a named parent,
		# adjusts the new node's parentIndex to match the new
		# node. The parented node is then inserted.
		parentIdx = 0
		for (i,node) in enumerate(self.nodes):
			if node.id == parent:
				parentIdx = i
		node.parentIndex = parentIdx
		self.nodes.append(node)

	def reparentById(self) -> None:
		# Scans the entire node tree, searching for parents
		# by name. Entries are re-mapped to match the named
		# parents. You can use this to build a tree from a
		# blob of raw data.
		for child in self.nodes:
			if child.parentId != "":
				for (i,node) in enumerate(self.nodes):
					if node.id == child.parentId:
						child.parentIndex = i

	def findNodeIndexById(self, id: str) -> int:
		# Finds a single node by identity(id)
		# Return -1 if not found
		for (i, node) in enumerate(self.nodes):
			if node.id == id:
				return i
		return -1

	def findNodeIndexByName(self, name: str) -> int:
		# Finds a single node by identity(name)
		# Return -1 if not found
		for (i, node) in enumerate(self.nodes):
			if node.displayName == name:
				return i
		return -1

	def findChildIndices(self, parentIndex: int) -> List:
		# Returns the indices of all nodes with a
		# parentIndex equal to the specified parameter
		result = []
		for (i, node) in enumerate(self.nodes):
			if node.parentIndex == parentIndex:
				result.append(i)
		return result

	def promoteClientsWithChildren(self) -> None:
		# Searches for client sites that have children,
		# and changes their node type to clientWithChildren
		for (i, node) in enumerate(self.nodes):
			if node.type == NodeType.client:
				for child in self.findChildIndices(i):
					if self.nodes[child].type != NodeType.device:
						node.type = NodeType.clientWithChildren

	def findUnconnectedNodes(self) -> List:
		# Performs a tree-traversal and finds any nodes that
		# aren't connected to the root. This is a "sanity check",
		# and also an easy way to handle "flat" topologies and
		# ensure that the unconnected nodes are re-connected to
		# the root.
		visited = []
		next = [0]

		while len(next) > 0:
			nextTraversal = next.pop()
			visited.append(nextTraversal)
			for idx in self.findChildIndices(nextTraversal):
				if idx not in visited:
					next.append(idx)

		result = []
		for i, n in enumerate(self.nodes):
			if i not in visited:
				result.append(i)
		return result

	def reconnectUnconnected(self):
		# Finds any unconnected nodes and reconnects
		# them to the root
		for idx in self.findUnconnectedNodes():
			if self.nodes[idx].type == NodeType.site:
				self.nodes[idx].parentIndex = 0
		for idx in self.findUnconnectedNodes():
			if self.nodes[idx].type == NodeType.clientWithChildren:
				self.nodes[idx].parentIndex = 0
		for idx in self.findUnconnectedNodes():
			if self.nodes[idx].type == NodeType.client:
				self.nodes[idx].parentIndex = 0

	def doesNetworkJsonExist(self):
		# Returns true if "network.json" exists, false otherwise
		import os
		return os.path.isfile("network.json")

	def __isSite(self, index) -> bool:
		return self.nodes[index].type == NodeType.ap or self.nodes[index].type == NodeType.site or self.nodes[index].type == NodeType.clientWithChildren

	def createNetworkJson(self):
		import json
		topLevelNode = {}
		self.__visited = [] # Protection against loops - never visit twice

		for child in self.findChildIndices(0):
			if child >0 and self.__isSite(child):
				topLevelNode[self.nodes[child].displayName] = self.__buildNetworkObject(child)

		del self.__visited

		with open('network.json', 'w') as f:
			json.dump(topLevelNode, f, indent=4)

	def __buildNetworkObject(self, idx):
		# Private: used to recurse down the network tree while building
		# network.json
		self.__visited.append(idx)
		node = {
			"downloadBandwidthMbps" : self.nodes[idx].downloadMbps,
			"uploadBandwidthMbps" : self.nodes[idx].uploadMbps,
		}
		children = {}
		hasChildren = False
		for child in self.findChildIndices(idx):
			if child >0 and self.__isSite(child) and child not in self.__visited:
				children[self.nodes[child].displayName] = self.__buildNetworkObject(child)
				hasChildren = True
		if hasChildren: node["children"] = children
		return node

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
		dot = graphviz.Digraph('network', comment = "Network Graph", engine="fdp")

		for (i, node) in enumerate(self.nodes):
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
				children = self.findChildIndices(i)
				for child in children:
					if child != i:
						if (self.nodes[child].type != NodeType.client and self.nodes[child].type != NodeType.device) or showClients:
							dot.edge("N" + str(i), "N" + str(child))

		dot.render("network.pdf")