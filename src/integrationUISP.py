"""Integration for syncing client service plans from UISP to LibreQoS"""

from pythonCheck import checkPythonVersion

checkPythonVersion()
import requests
import os
import csv
from datetime import datetime, timedelta
from integrationCommon import isIpv4Permitted, fixSubnet

try:
    from ispConfig import uispSite, uispStrategy, overwriteNetworkJSONalways, suspendedDownload, suspendedUpload, airMax_capacity
except ImportError:
    from ispConfig import uispSite, uispStrategy

    overwriteNetworkJSONalways = False


def uispRequest(target):
    """
    Sends an HTTP request to UISP and returns the result in JSON.
    You only need to specify the tail end of the URL, e.g. "sites"
    """
    from ispConfig import UISPbaseURL, uispAuthToken

    url = UISPbaseURL + "/nms/api/v2.1/" + target
    headers = {"accept": "application/json", "x-auth-token": uispAuthToken}
    r = requests.get(url, headers=headers, timeout=10)
    return r.json()

def buildFlatGraph():
    """Builds a high-performance (but lacking in site or AP bandwidth control) network."""
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
        siteType = site["identification"]["type"]
        if siteType == "endpoint":
            if site.get("ucrm"):
                _name = site["ucrm"]["client"]["name"]
                _custId = site["ucrm"]["client"]["id"]
                _serviceId = site["ucrm"]["service"]["id"]
            siteId = site["identification"]["id"]
            address = site["description"]["address"]
            customerName = _name
            customerId = _custId if not None else ""
            serviceId = _serviceId if not None else ""
            name = site["identification"]["name"]
            siteType = site["identification"]["type"]
            download = generatedPNDownloadMbps
            upload = generatedPNUploadMbps
            if (site["qos"]["downloadSpeed"]) and (site["qos"]["uploadSpeed"]):
                download = int(round(site["qos"]["downloadSpeed"] / 1000000))
                upload = int(round(site["qos"]["uploadSpeed"] / 1000000))
            if site.get("ucrm") and site["ucrm"]["service"]["status"] != 1:
                download = suspendedDownload
                upload = suspendedUpload

            node = NetworkNode(
                siteId=siteId,
                displayName=name,
                siteType=NodeType.client,
                download=download,
                upload=upload,
                address=address,
                customerName=customerName,
                customerId=customerId,
                serviceId=serviceId,
            )
            net.addRawNode(node)
            for device in devices:
                if (device["identification"]["site"] is not None and device["identification"]["site"]["id"] == siteId):
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
                    mac = device["identification"]["mac"]

                    net.addRawNode(
                        NetworkNode(
                            siteId=device["identification"]["id"],
                            displayName=device["identification"]["name"],
                            parentId=siteId,
                            siteType=NodeType.device,
                            ipv4=ipv4,
                            ipv6=ipv6,
                            mac=mac,
                        )
                    )

    # Finish up
    net.prepareTree()
    net.plotNetworkGraph(False)
    if net.doesNetworkJsonExist():
        if overwriteNetworkJSONalways:
            net.createNetworkJson()
        else:
            print(
                "network.json already exists and overwriteNetworkJSONalways set to False. Leaving in-place."
            )
    else:
        net.createNetworkJson()
    net.createShapedDevices()


def linkSiteTarget(link, direction):
    """Helper function to extract the site ID from a data link. Returns None if not present."""
    if link[direction]["site"] is not None:
        return link[direction]["site"]["identification"]["id"]

    return None


def findSiteLinks(dataLinks, siteId):
    """
        Searches the Data Links for any links to/from the specified site.
        Returns a list of site IDs that are linked to the specified site.
    """
    links = []
    for dl in dataLinks:
        fromSiteId = linkSiteTarget(dl, "from")
        if fromSiteId is not None and fromSiteId == siteId:
            # We have a link originating in this site.
            target = linkSiteTarget(dl, "to")
            if target is not None:
                links.append(target)

        toSiteId = linkSiteTarget(dl, "to")
        if toSiteId is not None and toSiteId == siteId:
            # We have a link originating in this site.
            target = linkSiteTarget(dl, "from")
            if target is not None:
                links.append(target)
    return links


def buildSiteBandwidths():
    """Builds a dictionary of site bandwidths from the integrationUISPbandwidths.csv file."""
    siteBandwidth = {}
    if os.path.isfile("integrationUISPbandwidths.csv"):
        with open("integrationUISPbandwidths.csv") as csv_file:
            csv_reader = csv.reader(csv_file, delimiter=",")
            next(csv_reader)
            for row in csv_reader:
                name, download, upload = row
                download = int(download)
                upload = int(upload)
                siteBandwidth[name] = {"download": download, "upload": upload}
    return siteBandwidth


def findApCapacities(devices, siteBandwidth):
    """Searches the UISP devices for APs and adds their capacities to the siteBandwidth dictionary."""
    for device in devices:
        if device["identification"]["role"] == "ap":
            name = device["identification"]["name"]
            if (
                name not in siteBandwidth
                and device["overview"]["downlinkCapacity"]
                and device["overview"]["uplinkCapacity"]
            ):
                safeToUse = True
                download = int(device["overview"]["downlinkCapacity"] / 1000000)
                upload = int(device["overview"]["uplinkCapacity"] / 1000000)
                if (download < 15) or (upload < 15):
                    print(
                        "WARNING: Device '"
                        + device["identification"]["hostname"]
                        + "' has unusually low capacity ("
                        + str(download)
                        + "/"
                        + str(upload)
                        + " Mbps). Discarding in favor of parent site rates."
                    )
                    safeToUse = False
                if device["identification"]["model"] == "WaveAP":
                    if (download < 500) or (upload < 500):
                        download = 2450
                        upload = 2450
                if safeToUse:
                    siteBandwidth[device["identification"]["name"]] = {
                        "download": download,
                        "upload": upload,
                    }


def airMaxCapacityCorrection(device, download, upload):
	dlRatio = None
	for interface in device['interfaces']:
		if ('wireless' in interface) and (interface['wireless'] != None):
			if 'dlRatio' in interface['wireless']:
				dlRatio = interface['wireless']['dlRatio']
	# UISP reports unrealistically high capacities for airMax.
	# For fixed frame, multiply capacity by the ratio for download/upload.
	# For Flexible Frame, use 65% of reported capcity.
	# 67/33
	if dlRatio == 67:
		download = download * 0.67
		upload = upload * 0.33
	# 50/50
	elif dlRatio == 50:
		download = download * 0.50
		upload = upload * 0.50
	# Flexible frame
	elif dlRatio == None:
		download = download * airMax_capacity
		upload = upload * airMax_capacity
	return (download, upload)

def findAirfibers(devices, generatedPNDownloadMbps, generatedPNUploadMbps):
    """Search UISP and find any AirFiber Devices."""
    foundAirFibersBySite = {}
    for device in devices:
        if device["identification"]["site"]["type"] == "site":
            if device["identification"]["role"] == "station":
                if device["identification"]["type"] == "airFiber":
                    if device["overview"]["status"] == "active":
                        if (
                            device["overview"]["downlinkCapacity"] is not None
                            and device["overview"]["uplinkCapacity"] is not None
                        ):
                            download = int(
                                device["overview"]["downlinkCapacity"] / 1000000
                            )
                            upload = int(device["overview"]["uplinkCapacity"] / 1000000)
                            # Make sure to factor in gigabit port for AF60/AF60-LRs
                            if (device["identification"]["model"] == "AF60-LR") or (
                                device["identification"]["model"] == "AF60"
                            ):
                                download = min(download, 950)
                                upload = min(upload, 950)
                            if (
                                device["identification"]["site"]["id"]
                                in foundAirFibersBySite
                            ):
                                if (
                                    download
                                    > foundAirFibersBySite[
                                        device["identification"]["site"]["id"]
                                    ]["download"]
                                ) or (
                                    upload
                                    > foundAirFibersBySite[
                                        device["identification"]["site"]["id"]
                                    ]["upload"]
                                ):
                                    foundAirFibersBySite[
                                        device["identification"]["site"]["id"]
                                    ]["download"] = download
                                    foundAirFibersBySite[
                                        device["identification"]["site"]["id"]
                                    ]["upload"] = upload
                                    # print(device['identification']['name'] + ' will override bandwidth for site ' + device['identification']['site']['name'])
                            else:
                                foundAirFibersBySite[
                                    device["identification"]["site"]["id"]
                                ] = {"download": download, "upload": upload}
    return foundAirFibersBySite

def buildSiteList(sites, dataLinks):
    """
        Builds a list of sites, including their IDs, names, and connections.
        Connections are determined by the dataLinks list.
    """
    siteList = []
    for site in sites:
        newSite = {
            "id": site["identification"]["id"],
            "name": site["identification"]["name"],
            "connections": findSiteLinks(dataLinks, site["identification"]["id"]),
            "cost": 10000,
            "parent": "",
            "type": type,
        }
        siteList.append(newSite)
    return siteList


def findInSiteList(siteList, name):
    """Searches the siteList for a site with the specified name."""
    for site in siteList:
        if site["name"] == name:
            return site
    return None


def findInSiteListById(siteList, id):
    """Searches the siteList for a site with the specified name."""
    for site in siteList:
        if site["id"] == id:
            return site
    return None


def debugSpaces(n):
    """Helper function to print n spaces."""
    spaces = ""
    for i in range(int(n)):
        spaces = spaces + " "
    return spaces


def walkGraphOutwards(siteList, root, routeOverrides):
    def walkGraph(node, parent, cost, backPath):
        site = findInSiteListById(siteList, node)
        routeName = parent["name"] + "->" + site["name"]
        if routeName in routeOverrides:
            # We have an overridden cost for this route, so use it instead
            # print("--> Using overridden cost for " + routeName + ". New cost: " + str(routeOverrides[routeName]) + ".")
            cost = routeOverrides[routeName]
        if cost < site["cost"]:
            # It's cheaper to get here this way, so update the cost and parent.
            site["cost"] = cost
            site["parent"] = parent["id"]
            # print(debugSpaces(cost/10) + parent['name'] + "->" + site['name'] + " -> New cost: " + str(cost))

            for connection in site["connections"]:
                if connection not in backPath:
                    # target = findInSiteListById(siteList, connection)
                    # print(debugSpaces((cost+10)/10) + site['name'] + " -> " + target['name'] + " (" + str(target['cost']) + ")")
                    newBackPath = backPath.copy()
                    newBackPath.append(site["id"])
                    walkGraph(connection, site, cost + 10, newBackPath)

    for connection in root["connections"]:
        # Force the parent since we're at the top
        site = findInSiteListById(siteList, connection)
        site["parent"] = root["id"]
        walkGraph(connection, root, 10, [root["id"]])


def loadRoutingOverrides():
    """Loads integrationUISProutes.csv and returns a set of "from", "to", "cost"."""
    overrides = {}
    if os.path.isfile("integrationUISProutes.csv"):
        with open("integrationUISProutes.csv", "r") as f:
            reader = csv.reader(f)
            for row in reader:
                if not row[0].startswith("#") and len(row) == 3:
                    fromSite, toSite, cost = row
                    overrides[fromSite.strip() + "->" + toSite.strip()] = int(cost)
    # print(overrides)
    return overrides


def findNodesBranchedOffPtMP(
    siteList, dataLinks, sites, rootSite, foundAirFibersBySite
):
    """Returns a list of nodes branched off a PtMP AP."""
    nodeOffPtMP = {}
    for site in siteList:
        id = site["id"]
        name = site["name"]
        if id != rootSite["id"]:
            if id not in foundAirFibersBySite:
                trueParent = findInSiteListById(siteList, id)["parent"]
                # parent = findInSiteListById(siteList, id)['parent']
                if site["parent"] is not None:
                    parent = site["parent"]
                    for link in dataLinks:
                        if (link["to"]["site"] is not None) and (
                            link["to"]["site"]["identification"] is not None):
                            if ("identification" in link["to"]["site"]) and (
                                link["to"]["site"]["identification"] is not None):
                                if ("identification" in link["from"]["site"]) and (
                                    link["from"]["site"]["identification"] is not None):
                                    # Respect parent defined by topology and overrides
                                    if (link["from"]["site"]["identification"]["id"] == trueParent):
                                        if (link["to"]["site"]["identification"]["id"] == id):
                                            if (link["from"]["device"]["overview"]["wirelessMode"] == "ap-ptmp"):
                                                if "overview" in link["to"]["device"]:
                                                    if ("downlinkCapacity" in link["to"]["device"]["overview"]) and (
                                                        "uplinkCapacity" in link["to"]["device"]["overview"]
                                                        ):
                                                        if (link["to"]["device"]["overview"]["downlinkCapacity"] is not None) and (
                                                            link["to"]["device"]["overview"]["uplinkCapacity"] is not None
                                                        ):
                                                            # Capacity of the PtMP client radio feeding the PoP will be used as the site bandwidth limit
                                                            download = int(round(link["to"]["device"]["overview"]["downlinkCapacity"] / 1000000))
                                                            upload = int(round(link["to"]["device"]["overview"]["uplinkCapacity"] / 1000000))
                                                            nodeOffPtMP[id] = {
                                                                "download": download,
                                                                "upload": upload,
                                                            }
                                                            site["parent"] = parent
                                                            if site["type"] == "site":
                                                                print(f"Site {name} will use PtMP AP as parent.")
    return siteList, nodeOffPtMP

def handleMultipleInternetNodes(sites, dataLinks, uispSite):
    """Handles multiple internet Nodes. Returns applicable sites, dataLinks, and uispSite."""
    internetConnectedSites = []
    for link in dataLinks:
        if link["canDelete"] == False:
            if (
                link["from"]["device"] is not None
                and link["to"]["device"] is not None
                and link["from"]["device"]["identification"]["id"]
                == link["to"]["device"]["identification"]["id"]
            ):
                siteID = link["from"]["site"]["identification"]["id"]
                # Found internet link
                internetConnectedSites.append(siteID)
    if len(internetConnectedSites) > 1:
        internetSite = {
            "id": "001",
            "identification": {
                "id": "001",
                "status": "active",
                "name": "Internet",
                "parent": None,
                "type": "site",
            },
        }
        sites.append(internetSite)
        uispSite = "Internet"
        for link in dataLinks:
            if link["canDelete"] == False:
                if (
                    link["from"]["device"]["identification"]["id"]
                    == link["to"]["device"]["identification"]["id"]
                ):
                    link["from"]["site"]["identification"]["id"] = "001"
                    link["from"]["site"]["identification"]["name"] = "Internet"
                    # Found internet link
                    internetConnectedSites.append(siteID)
        print("Multiple Internet links detected. Will create 'Internet' root node.")
    return (sites, dataLinks, uispSite)


def buildFullGraph():
    """
        Attempts to build a full network graph, incorporating as much of the UISP
        hierarchy as possible.
    """
    from integrationCommon import NetworkGraph, NetworkNode, NodeType
    from ispConfig import uispSite, generatedPNUploadMbps, generatedPNDownloadMbps

    # Load network sites
    print("Loading Data from UISP")
    sites = uispRequest("sites")
    devices = uispRequest("devices?withInterfaces=true&authorized=true")
    dataLinks = uispRequest("data-links?siteLinksOnly=true")

    # If multiple Internet-connected sites, create Internet root node:
    sites, dataLinks, uispSite = handleMultipleInternetNodes(sites, dataLinks, uispSite)

    # Build Site Capacities
    print("Compiling Site Bandwidths")
    siteBandwidth = buildSiteBandwidths()
    print("Finding AP Capacities")
    findApCapacities(devices, siteBandwidth)
    # Create a list of just network sites
    print("Creating list of sites")
    siteList = buildSiteList(sites, dataLinks)
    rootSite = findInSiteList(siteList, uispSite)
    print("Finding PtP Capacities")
    foundAirFibersBySite = findAirfibers(
        devices, generatedPNDownloadMbps, generatedPNUploadMbps
    )
    print("Creating list of route overrides")
    routeOverrides = loadRoutingOverrides()
    if rootSite is None:
        print("ERROR: Unable to find root site in UISP")
        return
    print("Walking graph outwards")
    walkGraphOutwards(siteList, rootSite, routeOverrides)
    print("Finding PtMP Capacities")
    siteList, nodeOffPtMP = findNodesBranchedOffPtMP(
        siteList, dataLinks, sites, rootSite, foundAirFibersBySite
    )

    # Debug code: dump the list of site parents
    # for s in siteList:
    # 	if s['parent'] == "":
    # 		p = "None"
    # 	else:
    # 		p = findInSiteListById(siteList, s['parent'])['name']
    # 		print(s['name'] + " (" + str(s['cost']) + ") <-- " + p)

    print("Building Topology")
    net = NetworkGraph()
    _custId = ""
    _serviceId = ""
    # Add all sites and client sites
    for site in sites:
        if site.get("ucrm"):
            _name = site["ucrm"]["client"]["name"]
            _custId = site["ucrm"]["client"]["id"]
            _serviceId = site["ucrm"]["service"]["id"]
        if site["identification"].get("parent"):
            _parent_id = site["identification"]["parent"]["id"]
        else:
            _parent_id = ""
        siteId = site["identification"]["id"]
        name = site["identification"]["name"]
        siteType = site["identification"]["type"]
        download = generatedPNDownloadMbps
        upload = generatedPNUploadMbps
        address = site["description"]["address"] if not None else None
        customerName = ""
        customerId = _custId if not None else ""
        serviceId = _serviceId if not None else ""
        parent = findInSiteListById(siteList, siteId)["parent"]
        if parent == "":
            if site["identification"]["parent"] is None:
                parent = ""
            else:
                parent = site["identification"]["parent"]["id"]
        if siteType == "site":
                nodeType = NodeType.site
                if name in siteBandwidth:
                    # Use the CSV bandwidth values
                    download = siteBandwidth[name]["download"]
                    upload = siteBandwidth[name]["upload"]
                else:
                    # Use limits from foundAirFibersBySite
                    if siteId in foundAirFibersBySite:
                        download = foundAirFibersBySite[siteId]["download"]
                        upload = foundAirFibersBySite[siteId]["upload"]
                    # If no airFibers were found, and node originates off PtMP, treat as child node of that PtMP AP
                    else:
                        if siteId in nodeOffPtMP:
                            if (nodeOffPtMP[siteId]["download"] >= download) or (
                                nodeOffPtMP[siteId]["upload"] >= upload
                            ):
                                download = nodeOffPtMP[siteId]["download"]
                                upload = nodeOffPtMP[siteId]["upload"]

                siteBandwidth[name] = {"download": download, "upload": upload}
        else:
                nodeType = NodeType.client
                address = site["description"]["address"]
                if site.get("ucrm"):
                    customerName = site["ucrm"]["client"]["name"]
                else:
                    customerName = ""
                if (site["qos"]["downloadSpeed"]) and (site["qos"]["uploadSpeed"]):
                    download = int(round(site["qos"]["downloadSpeed"] / 1000000))
                    upload = int(round(site["qos"]["uploadSpeed"] / 1000000))
                if site.get("ucrm") and site["ucrm"]["service"]["status"] != 1:
                    download = suspendedDownload
                    upload = suspendedUpload

        node = NetworkNode(
            siteId=siteId,
            displayName=name,
            siteType=nodeType,
            parentId=parent,
            download=download,
            upload=upload,
            address=address,
            customerName=customerName,
            customerId=customerId,
            serviceId=serviceId
        )
        # If this is the uispSite node, it becomes the root. Otherwise, add it to the
        # node soup.
        if name == uispSite:
            net.replaceRootNode(node)
        else:
            net.addRawNode(node)

        for device in devices:
            if (device["identification"]["site"] is not None and device["identification"]["site"]["id"] == siteId):
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
                mac = device["identification"]["mac"]

                net.addRawNode(
                    NetworkNode(
                        siteId=device["identification"]["id"],
                        displayName=device["identification"]["name"],
                        parentId=siteId,
                        siteType=NodeType.device,
                        ipv4=ipv4,
                        ipv6=ipv6,
                        mac=mac,
                    )
                )

    # Now iterate access points, and look for connections to sites
    for node in net.nodes:
        if node.siteType == NodeType.device:
            for dl in dataLinks:
                if (
                    dl["from"]["device"] is not None and dl["from"]["device"]["identification"]["id"] == node.siteId
                ):
                    if (
                        dl["to"]["site"] is not None and dl["from"]["site"]["identification"]["id"] != dl["to"]["site"]["identification"]["id"]
                    ):
                        target = net.findNodeIndexById(dl["to"]["site"]["identification"]["id"])
                        if target > -1:
                            # We found the site
                            if (
                                net.nodes[target].siteType == NodeType.client or net.nodes[target].siteType == NodeType.clientWithChildren
                            ):
                                net.nodes[target].parentId = node.siteId
                                node.type = NodeType.ap
                                if node.displayName in siteBandwidth:
                                    # Use the bandwidth numbers from the CSV file
                                    node.uploadMbps = siteBandwidth[node.displayName]["upload"]
                                    node.downloadMbps = siteBandwidth[node.displayName]["download"]
                                else:
                                    # Add some defaults in case they want to change them
                                    siteBandwidth[node.displayName] = {
                                        "download": generatedPNDownloadMbps,
                                        "upload": generatedPNUploadMbps,
                                    }

    net.prepareTree()
    print("Plotting network graph")
    net.plotNetworkGraph(False)
    if net.doesNetworkJsonExist():
        if overwriteNetworkJSONalways:
            net.createNetworkJson()
        else:
            print(
                "network.json already exists and overwriteNetworkJSONalways set to False. Leaving in-place."
            )
    else:
        net.createNetworkJson()
    net.createShapedDevices()

    # Save integrationUISPbandwidths.csv
    # (the newLine fixes generating extra blank lines)
    # Saves as .template as to not overwrite
    with open("integrationUISPbandwidths.template.csv", "w", newline="") as csvfile:
        wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
        wr.writerow(["ParentNode", "Download Mbps", "Upload Mbps"])
        for device in siteBandwidth:
            entry = (
                device,
                siteBandwidth[device]["download"],
                siteBandwidth[device]["upload"],
            )
            wr.writerow(entry)


def importFromUISP():
    """Import sites, devices, and service plans from UISP."""
    startTime = datetime.now()
    if uispStrategy == "full":
        buildFullGraph()
    else:
        buildFlatGraph()
    endTime = datetime.now()
    runTimeSeconds = ((endTime - startTime).seconds) + (
        ((endTime - startTime).microseconds) / 1000000
    )
    print(
        "UISP import completed in "
        + "{:g}".format(round(runTimeSeconds, 1))
        + " seconds"
    )


if __name__ == "__main__":
    importFromUISP()
