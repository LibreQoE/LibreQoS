# LibreQoS Integrations

If you need to create an integration for your network, we've tried to give you the tools you need. We currently ship integrations for UISP and Splynx. We'd love to include more.

### Overall Concept

LibreQoS enforces customer bandwidth limits, and applies CAKE-based optimizations at several levels:

* Per-user Cake flows are created. These require the maximum bandwidth permitted per customer.
    * Customers can have more than one device that share a pool of bandwidth. Customers are grouped into "circuits"
* *Optional* Access points can have a speed limit/queue, applied to all customers associated with the access point.
* *Optional* Sites can contain access points, and apply a speed limit/queue to all access points (and associated circuits).
* *Optional* Sites can be nested beneath other sites and access point, providing for a queue hierarchy that represents physical limitations of backhaul connections.

Additionally, you might grow to have more than one shaper - and need to express your network topology from the perspective of different parts of your network. (For example, if *Site A* and *Site B* both have Internet connections - you want to generate an efficient topology for both sites. It's helpful if you can derive this from the same overall topology)

LibreQoS's network modeling accomplishes this by modeling your network as a *graph*: a series of interconnected nodes, each featuring a "parent". Any "node" (entry) in the graph can be turned into a "root" node, allowing you to generate the `network.json` and `ShapedDevices.csv` files required to manage your customers from the perspective of that root node.

### Flat Shaping

The simplest form of integration produces a "flat" network. This is the highest performance model in terms of raw throughput, but lacks the ability to provide shaping at the access point or site level: every customer site is parented directly off the root.

> For an integration, it's recommended that you fetch the customer/device data from your management system rather than type them all in Python.

A flat integration is relatively simple. Start by importing the common API:

```python
from integrationCommon import isIpv4Permitted, fixSubnet, NetworkGraph, NetworkNode, NodeType
```

Then create an empty network graph (it will grow to represent your network):

```python
net = NetworkGraph()
```

Once you have your `NetworkGraph` object, you start adding customers and devices. Customers may have any number of devices. You can add a single customer with one device as follows:

```python
# Add the customer
customer = NetworkNode(
    id="Unique Customer ID",
    displayName="The Doe Family",
    type=NodeType.client,
    download=100, # Download is in Mbit/second
    upload=20, # Upload is in Mbit/second
    address="1 My Road, My City, My State")
net.addRawNode(customer) # Insert the customer ID

# Give them a device
device = NetworkNode(
    id="Unique Device ID", 
    displayName="Doe Family CPE",
    parentId="Unique Customer ID", # must match the customer's ID
    type=NodeType.device, 
    ipv4=["100.64.1.5/32"], # As many as you need, express networks as the network ID - e.g. 192.168.100.0/24
    ipv6=["feed:beef::12/64"], # Same again. May be [] for none.
    mac="00:00:5e:00:53:af"
)
net.addRawNode(device)
```

If the customer has multiple devices, you can add as many as you want - with `ParentId` continuing to match the parent customer's `id`.

Once you have entered all of your customers, you can finish the integration:

```python
net.prepareTree() # This is required, and builds parent-child relationships.
net.createNetworkJson() # Create `network.json`
net.createShapedDevices() # Create the `ShapedDevices.csv` file.
```

### Detailed Hierarchies

Creating a full hierarchy (with as many levels as you want) uses a similar strategy to flat networks---we recommend that you start by reading the "flat shaping" section above.

Start by importing the common API:

```python
from integrationCommon import isIpv4Permitted, fixSubnet, NetworkGraph, NetworkNode, NodeType
```

Then create an empty network graph (it will grow to represent your network):

```python
net = NetworkGraph()
```

Now you can start to insert sites and access points. Sites and access points are inserted like customer or device nodes: they have a unique ID, and a `ParentId`. Customers can then use a `ParentId` of the site or access point beneath which they should be located.

For example, let's create `Site_1` and `Site_2` - at the top of the tree:

```python
net.addRawNode(NetworkNode(id="Site_1", displayName="Site_1", parentId="", type=NodeType.site, download=1000, upload=1000))
net.addRawNode(NetworkNode(id="Site_2", displayName="Site_2", parentId="", type=NodeType.site, download=500, upload=500))
```

Let's attach some access points and point-of-presence sites:

```python
net.addRawNode(NetworkNode(id="AP_A", displayName="AP_A", parentId="Site_1", type=NodeType.ap, download=500, upload=500))
net.addRawNode(NetworkNode(id="Site_3", displayName="Site_3", parentId="Site_1", type=NodeType.site, download=500, upload=500))
net.addRawNode(NetworkNode(id="Site_5", displayName="Site_5", parentId="Site_3", type=NodeType.site, download=200, upload=200))        
net.addRawNode(NetworkNode(id="AP_9", displayName="AP_9", parentId="Site_5", type=NodeType.ap, download=120, upload=120))
net.addRawNode(NetworkNode(id="Site_6", displayName="Site_6", parentId="Site_5", type=NodeType.site, download=60, upload=60))
net.addRawNode(NetworkNode(id="AP_11", displayName="AP_11", parentId="Site_6", type=NodeType.ap, download=30, upload=30))
net.addRawNode(NetworkNode(id="Site_4", displayName="Site_4", parentId="Site_2", type=NodeType.site, download=200, upload=200))
net.addRawNode(NetworkNode(id="AP_7", displayName="AP_7", parentId="Site_4", type=NodeType.ap, download=100, upload=100))
net.addRawNode(NetworkNode(id="AP_1", displayName="AP_1", parentId="Site_2", type=NodeType.ap, download=150, upload=150))
```

When you attach a customer, you can specify a tree entry (e.g. `Site_5`) as a parent:

```python
# Add the customer
customer = NetworkNode(
    id="Unique Customer ID",
    displayName="The Doe Family",
    parentId="Site_5",
    type=NodeType.client,
    download=100, # Download is in Mbit/second
    upload=20, # Upload is in Mbit/second
    address="1 My Road, My City, My State")
net.addRawNode(customer) # Insert the customer ID

# Give them a device
device = NetworkNode(
    id="Unique Device ID", 
    displayName="Doe Family CPE",
    parentId="Unique Customer ID", # must match the customer's ID
    type=NodeType.device, 
    ipv4=["100.64.1.5/32"], # As many as you need, express networks as the network ID - e.g. 192.168.100.0/24
    ipv6=["feed:beef::12/64"], # Same again. May be [] for none.
    mac="00:00:5e:00:53:af"
)
net.addRawNode(device)
```

Once you have entered all of your network topology and customers, you can finish the integration:

```python
net.prepareTree() # This is required, and builds parent-child relationships.
net.createNetworkJson() # Create `network.json`
net.createShapedDevices() # Create the `ShapedDevices.csv` file.
```

You can also add a call to `net.plotNetworkGraph(False)` (use `True` to also include every customer; this can make for a HUGE file) to create a PDF file (currently named `network.pdf.pdf`) displaying your topology. The example shown here looks like this:

![](testdata/network_new.png)

### Per‑Circuit SQM Overrides (Download/Upload)

LibreQoS supports optional per‑circuit SQM overrides via the last column (`sqm`) of `ShapedDevices.csv`.

- Accepted values:
  - Single token: `cake`, `fq_codel`, or `none` (applies to both directions)
  - Directional: `down_sqm/up_sqm` where each side is one of `cake`, `fq_codel`, `none`, or empty
- Directional semantics:
  - Left token is download, right token is upload (e.g., `cake/fq_codel`)
  - Either side may be empty to leave that direction at default (e.g., `cake/` or `/fq_codel`)
  - `none` disables the SQM qdisc for that direction
- Normalization: Values are trimmed and lower‑cased; case is ignored on load
- Defaults and fast‑queues: If a side is unspecified (empty) or the entire field is empty, the global default SQM applies for that side. The “fast queues to fq_codel” threshold is evaluated per direction when no explicit override is set.

Examples:

```
# Both directions cake
...,cake

# Download cake, upload fq_codel
...,cake/fq_codel

# Download explicit (fq_codel), upload default
...,fq_codel/

# Disable upload SQM only
...,/none
```

## Longest Prefix Match Tip
You could theoretically throttle all unknown IPs until they are associated with a client. For example, you could limit every unknown to 1.5x0.5 with single entry in ShapedDevices.csv, until you associate them with an account. IPs need to be non-exact matches. So you can't have two 192.168.1.1 entries, but you can have a 192.168.1.0/24 subnet and a 192.168.1.2/32 - they aren't duplicates, and the LPM search is smart enough to pick the most exact match.
