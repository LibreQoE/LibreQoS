<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/banner2022.png"></a>

Learn more at [LibreQoS.io](https://libreqos.io/)

## System Requirements
* VM or physical server. Physical server will perform better and better utilize all CPU cores.
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
* Choose a CPU with solid single-thread performance within your budget. Generally speaking, any new CPU above $200 can probably handle shaping up to 2Gbps.
## Installation and Usage Guide

Best Performance, Bare Metal, IPv4 Only:
- 📄 [Alpha] [LibreQoS v1.2 Installation & Usage Guide Physical Server and Ubuntu 22.04](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.2-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-22.04)
- 📄 [Beta] [LibreQoS v1.1 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.1-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-21.10)
- 📄 [LibreQoS v1.0 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.0-Installation-&-Usage-Guide---Physical-Server-and-Ubuntu-21.10)
- 📄 [LibreQoS v0.9 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.9-Installation-&-Usage-Guide----Physical-Server-and-Ubuntu-21.10)

Good Performance, VM,  IPv4 Only:
- 📄 [LibreQoS v0.9 Installation & Usage Guide Proxmox and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.9-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-21.10)

OK Performance, IPv4 and IPv6:
- 📄 [LibreQoS 0.8 Installation and Usage Guide - Proxmox and Ubuntu 20.04 LTS](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.8-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-20.04)
