<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/banner2022.png"></a>

LibreQoS is a Quality of Experience and Smart Queue Management system designed for Internet Service Providers (such as Fixed Wireless Internet Service Providers) to optimize the flow of their network traffic and thus reduce bufferbloat, keep the network responsive, and improve the end-user experience.

Because customers see greater performance, ISPs receive fewer support tickets/calls and reduce network traffic from fewer retransmissions.

A sub-$1000 server running LibreQoS can shape traffic for hundreds or thousands of customers at over 10 Gbps. 

Learn more at [LibreQoS.io](https://libreqos.io/)

<img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/v1.1-alpha-preview.jpg"></a>

## Major Features
* Flexible Hierarchical Shaping / Back-haul Congestion Mitigation
  * Network hierarchy can be mapped to a json file in v1.1+. This allows for both simple network heirarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client). This allows operators to ensure a given site’s peak bandwidth will not exceed the capacity of its back-haul links (back-haul congestion control). This can allow operators to support more users on the same network equipment with LibreQoS than with competing QoE solutions which only shape by AP and Client. Shaping just by AP and client could allow for high aggregate peaks to occur on back-hauls links, which can trigger packet loss and disrupt network connectivity. LibreQoS’ flexible shaping provides a solution to this.
* CAKE – The Gold Standard of Queuing
  * CAKE is the product of nearly a decade of development efforts to improve on fq_codel. With the diffserv_4 parameter enabled – CAKE groups traffic in to Bulk, Best Effort, Video, and Voice. This means that without having to fine-tune traffic priorities as you would with DPI products – CAKE automatically ensures your clients’ OS update downloads will not disrupt their zoom calls. It allows for multiple video conferences to operate on the same connection which might otherwise “fight” for upload bandwidth causing call disruptions. It holds the connection together like glue. With work-from-home, remote learning, and tele-medicine becoming increasingly common – minimizing video call disruptions can save jobs, keep students engaged, and help ensure equitable access to medical care.
* XDP
  * Fast, multi-CPU queueing leveraging xdp-cpumap-tc. Tested up to 11 Gbps of real world traffic with just 30% CPU use on an Intel Xeon Gold 6254. Likely capable of 30Gbps or more.
* Graphing
  * Graph bandwidth by client and node (Site, AP, etc), with great visalizations made possible by InfluxDB

## System Requirements
* VM or physical server. Physical server will have higher throughput (XDP vs generic XDP).
* One management network interface, completely separate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* Dedicated Network Interface Card
* NIC must have two or more interfaces for traffic shaping.
* NIC must have multiple TX/RX transmit queues. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * Known supported cards:
    * [NVIDIA ConnectX-4 MCX4121A-XCAT](https://store.mellanox.com/products/nvidia-mcx4121a-xcat-connectx-4-lx-en-adapter-card-10gbe-dual-port-sfp28-pcie3-0-x8-rohs-r6.html)
    * [Intel X710](https://www.fs.com/products/75600.html)
    * Intel X520
* [Ubuntu Server 22.04](https://ubuntu.com/download/server) or above recommended. All guides assume Ubuntu Server 21.10 or above. Ubuntu Desktop is not recommended as it uses NetworkManager instead of Netplan.
* Kernel version 5.14 or above
* Python 3, PIP, and some modules (listed in respective guides).
* Choose a CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html) within your budget.
  * Recommendations for 10G+ throughput:
    * [AMD Ryzen 9 5900X](https://www.bestbuy.com/site/amd-ryzen-9-5900x-4th-gen-12-core-24-threads-unlocked-desktop-processor-without-cooler/6438942.p?skuId=6438942)
    * [Intel Core i7-12700KF](https://www.bestbuy.com/site/intel-core-i7-12700kf-desktop-processor-12-8p-4e-cores-up-to-5-0-ghz-unlocked-lga1700-600-series-chipset-125w/6483674.p?skuId=6483674)
    
## Versions
IPv4 + IPv6:
- [v1.2-alpha](https://github.com/rchac/LibreQoS/tree/main/v1.2)

IPv4 only:
- [v1.1-beta](https://github.com/rchac/LibreQoS/tree/main/v1.1)
- [v1.0-stable](https://github.com/rchac/LibreQoS/tree/main/v1.0)
