<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://user-images.githubusercontent.com/22501920/202913614-4ff2e506-e645-4a94-9918-d512905ab290.png"></a>

LibreQoS is a Quality of Experience (QoE) Smart Queue Management (SQM) system designed for Internet Service Providers to optimize the flow of their network traffic and thus reduce bufferbloat, keep the network responsive, and improve the end-user experience.

Servers running LibreQoS can shape traffic for many thousands of customers. 

Learn more at [LibreQoS.io](https://libreqos.io/)!

## Support LibreQoS

Please support the continued development of LibreQoS by visiting our [GitHub Sponsors page](https://github.com/sponsors/LibreQoE).

## Features
### Flexible Hierarchical Shaping / Back-Haul Congestion Mitigation
<img src="https://raw.githubusercontent.com/LibreQoE/LibreQoS/main/docs/nestedHTB2.png" width="350"></img>

Starting in version v1.1+, operators can map their network hierarchy in LibreQoS. This enables both simple network hierarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client). This can be used to ensure that a given site’s peak bandwidth will not exceed the capacity of its back-haul links (back-haul congestion control). Operators can support more users on the same network equipment with LibreQoS than with competing QoE solutions which only shape by AP and Client.

### CAKE
CAKE is the product of nearly a decade of development efforts to improve on fq_codel. With the diffserv_4 parameter enabled – CAKE groups traffic in to Bulk, Best Effort, Video, and Voice. This means that without having to fine-tune traffic priorities as you would with DPI products – CAKE automatically ensures your clients’ OS update downloads will not disrupt their zoom calls. It allows for multiple video conferences to operate on the same connection which might otherwise “fight” for upload bandwidth causing call disruptions. It holds the connection together like glue. With work-from-home, remote learning, and tele-medicine becoming increasingly common – minimizing video call disruptions can save jobs, keep students engaged, and help ensure equitable access to medical care.

### XDP
Fast, multi-CPU queueing leveraging xdp-cpumap-tc and cpumap-pping. Currently tested in the real world past 11 Gbps (so far) with just 30% CPU use on a 16 core Intel Xeon Gold 6254. It's likely capable of 30Gbps or more.

### Graphing
You can graph bandwidth and TCP RTT by client and node (Site, AP, etc), with great visalizations made possible by InfluxDB.
<img alt="LibreQoS" src="https://raw.githubusercontent.com/LibreQoE/LibreQoS/main/docs/v1.1-alpha-preview.jpg"></a>

### CRM Integrations
* UISP
* Splynx

## System Requirements
### VM or physical server
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
- [v1.3](https://github.com/LibreQoE/LibreQoS/tree/main/v1.3) [Setup Guide](https://github.com/LibreQoE/LibreQoS/wiki/LibreQoS-v1.3-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-22.04)
