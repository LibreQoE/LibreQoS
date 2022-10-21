<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/banner2022-2.png"></a>

LibreQoS is a Quality of Experience and Smart Queue Management system designed for Internet Service Providers (such as Fixed Wireless Internet Service Providers) to optimize the flow of their network traffic and thus reduce bufferbloat, keep the network responsive, and improve the end-user experience.

Because customers see greater performance, ISPs receive fewer support tickets/calls and reduce network traffic from fewer retransmissions.

Servers running LibreQoS can shape traffic for many thousands of customers. 

Learn more at [LibreQoS.io](https://libreqos.io/)

<img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/v1.1-alpha-preview.jpg"></a>

## Real World Impact
By allowing ISPs to better optimize traffic flows – LibreQoS can improve the reliability of end-users’ voice and video calls transiting the network. With work-from-home, remote learning, and tele-medicine becoming increasingly common – it is important to minimize any disruptions to video calls and VoIP that might otherwise occur due to bufferbloat within the ISP network. LibreQoS mitigates such bufferbloat, which can have important real world benefits for end-users, such as:

* Keeping remote workers productive and employed
* Mitigating learning disruptions – keeping students engaged and on-track with their peers
* Reduce educational and employment inequities for people with disabilities
* Allowing for reliable access to tele-medicine

## Features
### Flexible Hierarchical Shaping
<img src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/nestedHTB2.png" width="350"></img>

Network hierarchy can be mapped to a json file in v1.1+. This allows for both simple network heirarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client). This allows operators to ensure a given site’s peak bandwidth will not exceed the capacity of its back-haul links (back-haul congestion control). This can allow operators to support more users on the same network equipment with LibreQoS than with competing QoE solutions which only shape by AP and Client. Shaping just by AP and client could allow for high aggregate peaks to occur on back-hauls links, which can trigger packet loss and disrupt network connectivity. LibreQoS’ flexible shaping provides a solution to this.

### CAKE
CAKE is the product of nearly a decade of development efforts to improve on fq_codel. With the diffserv_4 parameter enabled – CAKE groups traffic in to Bulk, Best Effort, Video, and Voice. This means that without having to fine-tune traffic priorities as you would with DPI products – CAKE automatically ensures your clients’ OS update downloads will not disrupt their zoom calls. It allows for multiple video conferences to operate on the same connection which might otherwise “fight” for upload bandwidth causing call disruptions. It holds the connection together like glue.

### XDP
Fast, multi-CPU queueing leveraging xdp-cpumap-tc. Tested up to 11 Gbps of real world traffic with just 30% CPU use on an Intel Xeon Gold 6254. Likely capable of 30Gbps or more.

### Graphing
Graph bandwidth by client and node (Site, AP, etc), with great visalizations made possible by InfluxDB

### CRM Integrations
* UISP (v1.2+)
* Splynx (v1.3+)

## System Requirements
* VM or physical server
  * For VMs, NIC passthrough is required for optimal throughput and latency (XDP vs generic XDP). Using Virtio / bridging is much slower than NIC passthrough. Virtio / bridging should not be used for large amounts of traffic.

### CPU
* 2 or more CPU cores
* A CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html) within your budget.
  * For 10G+ throughput on a budget, cosnider the [AMD Ryzen 9 5900X](https://www.bestbuy.com/site/amd-ryzen-9-5900x-4th-gen-12-core-24-threads-unlocked-desktop-processor-without-cooler/6438942.p?skuId=6438942) or [Intel Core i7-12700KF](https://www.bestbuy.com/site/intel-core-i7-12700kf-desktop-processor-12-8p-4e-cores-up-to-5-0-ghz-unlocked-lga1700-600-series-chipset-125w/6483674.p?skuId=6483674)
* Recommended CPU cores assuming [single thread](https://www.cpubenchmark.net/singleThread.html) performance of 2700 or more:

| Throughput    | CPU Cores     |
| ------------- | ------------- |
| 1 Gbps        | 4             |
| 5 Gbps        | 8             |
| 10 Gbps       | 12            |
| 20 Gbps       | 16            |

### Memory
* Mimumum RAM = 2 + (0.002 x Subscriber Count) GB
* Recommended RAM:

| Subscribers   | RAM           |
| ------------- | ------------- |
| 100           | 4 GB          |
| 1,000         | 8 GB          |
| 5,000         | 16 GB         |
| 10,000        | 32 GB         |

### Network Interfaces
* One management network interface completely separate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* Dedicated Network Interface Card for Shaping Interfaces
  * NIC must have 2 or more interfaces for traffic shaping.
  * NIC must have multiple TX/RX transmit queues. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * Known supported cards:
    * [NVIDIA Mellanox MCX512A-ACAT](https://www.fs.com/products/119649.html)
    * [Intel X710](https://www.fs.com/products/75600.html)
    * Intel X520
    
## Versions
### IPv4 + IPv6
- [v1.2.1-stable](https://github.com/rchac/LibreQoS/tree/main/v1.2)
- [v1.3-alpha](https://github.com/rchac/LibreQoS/tree/main/v1.3)

### IPv4 only
- [v1.1](https://github.com/rchac/LibreQoS/tree/main/v1.1)
- [v1.0](https://github.com/rchac/LibreQoS/tree/main/v1.0)
