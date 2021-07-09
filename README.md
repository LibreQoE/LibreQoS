# LibreQoS
![Banner](docs/Banner.png "Banner")
LibreQoS is an application that allows ISPs to apply bandwidth rate limiting to hundreds of clients through cake or fq_codel. <a href="https://www.bufferbloat.net/projects/codel/wiki/Cake/">Cake</a> and <a href="https://www.bufferbloat.net/projects/codel/wiki/">fq_codel</a> are Free and Open Source Active Queue Management algorithms that reduce <a href="https://www.bufferbloat.net/projects/bloat/wiki/Introduction/">bufferbloat</a>. When used in the context of an ISP network, these AQMs can be deployed to shape traffic on each customer's connection - reducing latency, enforcing advertised plan bandwidth, and improving network performance. LibreQoS directs each customer's traffic through an individual cake or fq_codel instance, which acts as part of a <a href="https://linux.die.net/man/8/tc-htb">hierarchy token bucket</a>. Traffic can be shaped by site or by Access Point, in addition to by subscriber. Please test to ensure compatability with your network architecture and design before deploying in production. 
## Who is LibreQoS for?
This software is intended for Internet Service Providers. Internet Service Providers with more than 1000 subscribers may benefit more from using commercially supported alternatives with NMS/CRM integrations such as <a href="https://preseem.com/">Preseem</a> or <a href="https://www.saisei.com/">Saisei</a>.
```
╔══════════╦══════╦═══════════╦══════════╦══════╦═════╦═════════╦══════════════════╦══════════════════╗
║          ║ IPv4 ║ IPv6      ║ fq_codel ║ cake ║ DPI ║ Metrics ║ Shape By         ║ Throughput       ║
╠══════════╬══════╬═══════════╬══════════╬══════╬═════╬═════════╬══════════════════╬══════════════════╣
║ LibreQoS ║ ✔    ║ v0.8 only ║ ✔        ║ ✔    ║     ║         ║ AP, Client       ║ 10G+ (v0.9 only) ║
╠══════════╬══════╬═══════════╬══════════╬══════╬═════╬═════════╬══════════════════╬══════════════════╣
║ Preseem  ║ ✔    ║ ✔         ║ ✔        ║      ║     ║ ✔       ║ Site, AP, Client ║ 20G+             ║
╠══════════╬══════╬═══════════╬══════════╬══════╬═════╬═════════╬══════════════════╬══════════════════╣
║ Seisei   ║ ✔    ║ ✔         ║ ?        ║ ?    ║ ✔   ║ ✔       ║ ?                ║ 10G              ║
╚══════════╩══════╩═══════════╩══════════╩══════╩═════╩═════════╩══════════════════╩══════════════════╝
```
Individuals wanting to reduce bufferbloat or latency on their home internet connections may want to try a home router supporting fq_codel, such as Ubiquiti's EdgeRouter-X (must enable advanced queue fq_codel).

## How does fq_codel work?
Fq_codel distinguishes interactive flows of traffic (web browsing, audio streaming, VoIP, gaming) from bulk traffic (streaming video services, software updates). Interactive flows are prioritized to optimize their performance, while bulk traffic gets steady throughput and variable latency. The general reduction of connection latency offered by fq_codel is highly beneficial to end-users.

<img src="docs/latency.png" width="650">

The impact of fq_codel on a 3000Mbps connection vs hard rate limiting — a 30x latency reduction.
>“FQ_Codel provides great isolation... if you've got low-rate videoconferencing and low rate web traffic they never get dropped. A lot of issues with IW10 go away, because all the other traffic sees is the front of the queue. You don't know how big its window is, but you don't care because you are not affected by it. FQ_Codel increases utilization across your entire networking fabric, especially for bidirectional traffic... If we're sticking code into boxes to deploy codel, don't do that. Deploy fq_codel. It's just an across the board win.”
> - Van Jacobson | IETF 84 Talk
## Typical Client Results
Here are the <a href="http://www.dslreports.com/speedtest">DSLReports Speed Test</a> results for a Fixed Wireless client averaging 20ms to the test server.
Bloat is below 5ms in each direction.

<img src="docs/bloat.png" width="350">

# Network Design
* Edge and Core routers with MTU 1500 on links between them
   * If you use MPLS, you would terminate MPLS traffic at the core router. LibreQoS cannot decapsulate MPLS on its own.
* OSPF primary link (low cost) through the server running LibreQoS
* OSPF backup link

![Diagram](docs/design.png?raw=true "Diagram")

# v0.8 (Stable)
## Features
* Dual stack: client can be shaped by same qdisc for both IPv4 and IPv6
* Up to 1000 clients (IPv4/IPv6)
* Real world asymmetrical throughput: between 2Gbps and 4.5Gbps depending on CPU single thread performance. 
* HTB+fq_codel or HTB+cake
* Shape Clients by Access Point / Node capacity
* TC filters split into groups through hashing filters to increase throughput
* Simple client management via csv file
* Simple statistics - table shows top 20 subscribers by packet loss, with APs listed
## Limitations
* Qdisc locking problem limits throughput of HTB used in v0.8 (solved in v0.9). Tested up to 4Gbps/500Mbps asymmetrical throughput using <a href="https://github.com/microsoft/ethr">Microsoft Ethr</a> with n=500 streams. High quantities of small packets will reduce max throughput in practice.
* Linux tc hash tables can only handle <a href="https://stackoverflow.com/questions/21454155/linux-tc-u32-filters-strange-error">~4000 rules each</a>. This limits total possible clients to 1000 in v0.8.

# v0.9 (Beta/testing)
## Features
* <a href="https://github.com/xdp-project/xdp-cpumap-tc">XDP-CPUMAP-TC</a> integration greatly improves throughput, allows many more IPv4 clients, and lowers CPU use. Latency reduced by half on networks previously limited by single-CPU / TC QDisc locking problem in v.0.8.
* Tested up to 10Gbps asymmetrical throughput on dedicated server (lab only had 10G router). v0.9 is estimated to be capable of an asymmetrical throughput of 20Gbps-40Gbps on a dedicated server with 12+ cores.
* ![Throughput](docs/10Gbps.png?raw=true "Throughput")
* MQ+HTB+fq_codel or MQ+HTB+cake
* Shape Clients by Access Point / Node capacity
* APs equally distributed among CPUs / NIC queues to greatly increase throughput
* Simple client management via csv file
## Limitations
* Not dual stack, clients can only be shaped by IPv4 address for now in v0.9. Once IPv6 support is added to <a href="https://github.com/xdp-project/xdp-cpumap-tc">XDP-CPUMAP-TC</a> we can then shape IPv6 as well.
* XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).
* Working on stats feature
## Requirements
* Requires kernel version 5.12 or above for physical servers, and kernel version 5.14 or above for VM.

# General Requirements
* VM or physical server
* One management network interface, completely seperate from the traffic shaping interfaces.
* NIC supporting two interfaces for traffic shaping
  * <a href="https://store.mellanox.com/categories/products/adapter-cards.html?_bc_fsnf=1&Technology=Ethernet&Ports=Dual">NVIDIA ConnectX</a>
  * <a href="https://www.fs.com/products/75600.html">Intel X710</a>
* Ubuntu Server recommended. Ubuntu Desktop is not recommended as it uses NetworkManager instead of Netplan.
* Python 3, PIP, and some modules (listed in respective guides).
* Choose a CPU with solid <a href="https://www.cpubenchmark.net/singleThread.html">single-thread performance</a> within your budget. Generally speaking any new CPU above $200 can probably handle shaping up to 2Gbps.


## Installation and Usage Guide
📄 <a href="https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.9-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-21.10">LibreQoS 0.9 Installation and Usage Guide - Proxmox and Ubuntu 21.10</a>

📄 <a href="https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.8-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-20.04">LibreQoS 0.8 Installation and Usage Guide - Proxmox and Ubuntu 20.04 LTS</a>

## Donate
LibreQoS makes great use of fq_codel - an open source project led by Dave Taht, and contrinuted to by dozens of others. Without Dave's work, there would be no LibreQoS, Preseem, or Saisei. Please contribute to Dave's patreon here: https://www.patreon.com/dtaht

If this application helps your network, please consider donating to Dave's patreon. Donating just $0.2/sub/month ($100/month for 500 subs) comes out to be 60% less than any proprietary solution, and you get to ensure continued development of fq_codel's successor, CAKE.

## Special Thanks
Special thanks to Dave Taht, Jesper Dangaard Brouer, Toke Høiland-Jørgensen, Kumar Kartikeya Dwivedi, Maxim Mikityanskiy, Yossi Kuperman, and Rony Efraim for their many contributions to the linux networking stack. Thank you Phil Sutter, Bert Hubert, Gregory Maxwell, Remco van Mook, Martijn van Oosterhout, Paul B Schroeder, and Jasper Spaans for contributing to the guides and documentation listed below. Thanks to Leo Manuel Magpayo for his help improving documentation and for testing. Thanks to everyone on the <a href="https://lists.bufferbloat.net/listinfo/">Bufferbloat mailing list</a> for your help and contibutions.

## References
* https://tldp.org/HOWTO/Adv-Routing-HOWTO/lartc.adv-filter.hashing.html
* http://linux-ip.net/gl/tc-filters/tc-filters.html
* http://linux-tc-notes.sourceforge.net/tc/doc/cls_u32.txt
* https://netdevconf.info/0x14/pub/papers/44/0x14-paper44-talk-paper.pdf

## License
Copyright (C) 2020-2021 Robert Chacón

LibreQoS is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 2 of the License, or
(at your option) any later version.

LibreQoS is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with LibreQoS.  If not, see <http://www.gnu.org/licenses/>.
