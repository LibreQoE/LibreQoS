<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/banner2022-2.png"></a>

LibreQoS is a Quality of Experience (QoE) Smart Queue Management (SQM) system designed for Internet Service Providers to optimize the flow of their network traffic and thus reduce bufferbloat, keep the network responsive, and improve the end-user experience.

Servers running LibreQoS can shape traffic for many thousands of customers. 

Learn more at [LibreQoS.io](https://libreqos.io/)!

## Features
### Flexible Hierarchical Shaping / Back-Haul Congestion Mitigation
<img src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/nestedHTB2.png" width="350"></img>

Your network hierarchy is mapped to a json file. This allows for both simple network hierarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client). This allows operators to ensure a given site’s peak bandwidth will not exceed the capacity of its back-haul links (back-haul congestion control). This can allow operators to support more users on the same network equipment with LibreQoS than with competing QoE solutions which only shape by AP and Client. Shaping just by AP and client could allow for high aggregate peaks to occur on back-hauls' links, which can trigger packet loss and disrupt network connectivity. LibreQoS’s flexible shaping provides a solution to this.

### CAKE
CAKE is the product of a decade of development efforts to improve on [fq_codel](https://www.rfc-editor.org/rfc/rfc8290.html). With the diffserv4 parameter enabled – CAKE groups traffic into Bulk, Best Effort, Video, and Voice "tins" that closely match the relevant IETF diffserv standards ([RFC4594](https://www.rfc-editor.org/rfc/rfc4594.html), [RFC7567](https://www.rfc-editor.org/rfc/rfc7657), and [RFC8622](https://datatracker.ietf.org/doc/html/rfc8622)). This means that without having to fine-tune traffic priorities as you would with DPI products – CAKE automatically ensures your clients’ OS update downloads will not disrupt their zoom calls. It allows for multiple video conferences to operate on the same connection which might otherwise “fight” for upload bandwidth causing call disruptions.

### XDP
Fast, multi-CPU queueing leveraging xdp-cpumap-tc. Currently tested in the real world past 11 Gbps (so far) with just 30% CPU use on a 16 core Intel Xeon Gold 6254. It's likely capable of 30Gbps or more.

### Graphing
You can graph bandwidth by client and node (Site, AP, etc), with great visalizations made possible by InfluxDB.
<img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/v1.1-alpha-preview.jpg"></a>

### CRM Integrations
* UISP
* Splynx

## System Requirements
* VM or physical server
  * For VMs, NIC passthrough is required for optimal throughput and latency (XDP vs generic XDP). Using Virtio / bridging is much slower than NIC passthrough. Virtio / bridging should not be used for large amounts of traffic.

### CPU
* 2 or more CPU cores
* A CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html) within your budget.
  * For 10G+ throughput on a budget, consider the [AMD Ryzen 9 5900X](https://www.bestbuy.com/site/amd-ryzen-9-5900x-4th-gen-12-core-24-threads-unlocked-desktop-processor-without-cooler/6438942.p?skuId=6438942) or [Intel Core i7-12700KF](https://www.bestbuy.com/site/intel-core-i7-12700kf-desktop-processor-12-8p-4e-cores-up-to-5-0-ghz-unlocked-lga1700-600-series-chipset-125w/6483674.p?skuId=6483674)
* CPU Core count required assuming [single thread](https://www.cpubenchmark.net/singleThread.html) performance of 2700 or more:

| Throughput    | CPU Cores     |
| ------------- | ------------- |
| 500 Mbps      | 2             |
| 1 Gbps        | 4             |
| 5 Gbps        | 8             |
| 10 Gbps       | 12            |
| 20 Gbps*      | 16            |
| 50 Gbps*      | 32            |
| 100 Gbps*     | 64            |

(* Estimated)

### Memory
* Mimumum RAM = 2 + (0.002 x Subscriber Count) GB
* Recommended RAM:

| Subscribers   | RAM           |
| ------------- | ------------- |
| 100           | 4 GB          |
| 1,000         | 8 GB          |
| 5,000         | 16 GB         |
| 10,000*       | 32 GB         |
| 50,000*       | 48 GB         |

(* Estimated)

### Network Interface Requirements
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
   - [Setup Guide](https://github.com/LibreQoE/LibreQoS/wiki/LibreQoS-v1.2-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-22.04)
- [v1.3-beta](https://github.com/rchac/LibreQoS/tree/main/v1.3)
   - [Setup Guide](https://github.com/LibreQoE/LibreQoS/wiki/LibreQoS-v1.3-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-22.04)
