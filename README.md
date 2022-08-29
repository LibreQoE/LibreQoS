# LibreQoS
![Banner](docs/Banner.png "Banner")
LibreQoS is a Smart Queue Management system designed for Internet Service Providers (such as Fixed Wireless 
Internet Service Providers) to optimize the flow of their customers' traffic and thus improve the 
end-user experience, 
prevent [bufferbloat,](https://www.bufferbloat.net/projects/bloat/wiki/Introduction/) 
and keep the network responsive.

Because the customers see better performance, ISPs receive fewer support 
tickets/calls and reduce network traffic from fewer retransmissions.

A sub-$1000 computer running LibreQoS can shape traffic for
hundreds or thousands of customers at up to 10 Gbps.

# How does LibreQoS work?

ISPs use LibreQoS to enforce customer plan bandwidth, improve responsiveness,
reduce latency, reduce jitter, reduce bufferbloat, and improve overall network performance. 

LibreQoS runs on a computer that sits between your upstream provider and the
core of your network (see graphic below).
It manages all customer traffic with the
[htb+cake](https://www.bufferbloat.net/projects/codel/wiki/Cake/)
or [htb+fq\_codel](https://www.bufferbloat.net/projects/codel/wiki/)
Active Queue Management (AQM) algorithms.

LibreQoS directs each customer's traffic into a
[hierarchy token bucket](https://linux.die.net/man/8/tc-htb),
where traffic can be shaped by the subscriber's allocated
plan bandwidth, as well as by any upstream constraints within
the ISP network (Access Point capacity, backhaul capacity, etc).

## Who should use LibreQoS?

**The target for LibreQoS is ISPs** that have a modest number of subscribers (<2000).
LibreQoS runs on an inexpensive computer and handles up to thousands of subscribers.

**Individuals** can reduce bufferbloat or latency on their home internet connections
(whether or not their service provider offers an AQM solution)
with a router that supports fq\_codel, such as
[IQrouter](https://evenroute.com),
[Ubiquiti's EdgeRouter-X](https://www.ui.com/edgemax/edgerouter-x/) (be sure to enable *advanced queue fq\_codel*),
or installing [OpenWrt](https://openwrt.org) or [DD-WRT](https://dd-wrt.com) on their existing router.

**Large Internet Service Providers** with significantly more subscribers may
benefit from using commercially supported alternatives with NMS/CRM integrations
such as [Preseem](https://preseem.com) or [Saisei](https://www.saisei.com/).
See the table below.

**A comparison of LibreQoS and Preseem**

```
╔══════════════════════╦══════════════════════╦══════════════════╗
║ Feature              ║ LibreQoS             ║ Preseem          ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ IPv4                 ║ ✔                    ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ IPv6                 ║ v0.8 only            ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ fq_codel             ║ ✔                    ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ cake                 ║ ✔                    ║                  ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ Fair Queuing         ║ ✔                    ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ VoIP Prioritization  ║ ✔ cake diffserv4 [1] ║                  ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ Video Prioritization ║ ✔ cake diffserv4 [1] ║                  ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ CRM Integration      ║                      ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ Metrics              ║                      ║ ✔                ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ Shape By             ║ Site, AP, Client     ║ AP, Client       ║
╠══════════════════════╬══════════════════════╬══════════════════╣
║ Throughput           ║ 11G+ (v0.9+)         ║ 20G+ [2]         ║
╚══════════════════════╩══════════════════════╩══════════════════╝
```
* [1] [Piece of CAKE: A Comprehensive Queue Management Solution for Home Gateways](https://arxiv.org/pdf/1804.07617.pdf)
* [2] [Aterlo Validates QoE Measurement Appliance Preseem](https://www.cengn.ca/wp-content/uploads/2020/02/Aterlo-Networks-Success-Story.pdf)

## Why not just use Mikrotik Queues in ROS v7?
* Mikrotik's ROS v7 cannot make use of XDP based hardware acceleration, so deeply nested HTBs with high throughput (shaping by backhaul, Site, AP, client) will not always be viable. Queuing with CAKE is still not fully working, though they're making good progress with fq_codel.
* A middle-box x86 device running LibreQoS can put through up to 4Gbps of traffic per CPU core, allowing you to shape by a Site or AP up to 4Gbps in capacity, with aggregate throughput of 10Gbps or more with multiple cores. On ARM-based MikroTik routers, the most traffic you can put through a single HTB and CPU core is probably closer to 2Gbps. By defauly, linux HTBs suffer from queue locking, where CPU use will look as if all CPU cores are evenly balancing the load, but in reality, a single Qdisc lock on the first CPU core (which handles scheduling of the other cpu threads) will be the bottleneck of all HTB throughput. The way LibreQoS works around that qdisc locking problem is with XDP-CPUMAP-TC, which uses XDP and MQ to run a separate HTB instance on each CPU core. That is not available on MikroTik. Heirarchical queuing is bottle-necked on Mikrotik in this way.
* Routing on the same device which applies CPU-intensive queues such as fq-codel and CAKE will greatly increase CPU use, limiting bandwidth throughput and introducing more potential latency and jitter for end-users than would be seen using a middle-box such as LibreQoS.

## Why not just use Preseem or Paraqum?
* Preseem and Paraqum are great commercial products - certainly consider them if you want the features and support they provide.
* That said, the monthly expense of those programs could instead be put toward the active development of CAKE and fq_codel, the AQMs which are the underlying algorithms that make Preseem and Paraqum possible. For example, Dave Täht is one of the leading figures of the bufferbloat project. He currently works to improve implementations of fq_codel and CAKE, educate others about bufferbloat, and advocate for the standardization of those AQMs on hardware around the world. Every dollar contributed to Dave's patreon will come back to ISPs 10-fold with improvements to fq_codel, CAKE, and the broader internet in general. If your ISP has benefited from LibreQoS, Preseem, or Paraqum, please [contribute to Dave's Patreon here.](https://www.patreon.com/dtaht) Our goal is to get Dave's patreon to $5000 per month - so he can focus on CAKE and fq_codel full-time, especially on ISP-centric improvements. Just 50 ISPs contributing $100/month will make it happen.

## How do Cake and fq\_codel work?

CAKE and fq_codel are hybrid packet scheduler and Active Queue Management (AQM) algorithms. LibreQoS uses a Hierarchical token bucket (HTB) to direct each customer's traffic into its own queue, where it is then shaped using either CAKE or fq_codel. Each customer's bandwidth ceiling is controlled by the HTB, according to the customer's allocated plan bandwidth, as well as the available capacity of the customer's respective Access Point and Site.

The difference is dramatic: the chart below shows the ping times during a
[Realtime Response Under Load (RRUL) test](https://www.bufferbloat.net/projects/bloat/wiki/RRUL_Chart_Explanation/)
before and after enabling LibreQoS AQM.
The RRUL test sends full-rate traffic in both directions, then measures latency
during the transfer.
Note that the latency drops from ~20 msec (green, no LibreQoS) to well
under 1 msec (brown, using LibreQoS).

<img src="docs/latency.png" width="650">

The impact of fq\_codel on a 3000Mbps connection vs hard rate limiting —
a 30x latency reduction.
>“FQ\_Codel provides great isolation... if you've got low-rate videoconferencing and low rate web traffic they never get dropped. A lot of issues with IW10 go away, because all the other traffic sees is the front of the queue. You don't know how big its window is, but you don't care because you are not affected by it. FQ\_Codel increases utilization across your entire networking fabric, especially for bidirectional traffic... If we're sticking code into boxes to deploy codel, don't do that. Deploy fq\_codel. It's just an across the board win.”
> - Van Jacobson | IETF 84 Talk

**References**

* [Cake | Bufferbloat.net](https://www.bufferbloat.net/projects/codel/wiki/Cake/)
* [FQ-Codel | Bufferbloat.net](https://www.bufferbloat.net/projects/codel/wiki/)

## Typical Client Results
Here are the [DSLReports Speed Test](http://www.dslreports.com/speedtest)
results for a Fixed Wireless client averaging 20ms to the test server.
LibreQoS keeps added latency below 5ms in each direction.

<img src="docs/bloat.png" width="350">

# Network Design
* Edge and Core routers with MTU 1500 on links between them
   * If you use MPLS, you would terminate MPLS traffic at the core router.
LibreQoS cannot decapsulate MPLS on its own.
* OSPF primary link (low cost) through the server running LibreQoS
* OSPF backup link

![Diagram](docs/design.png?raw=true "Diagram")

### v0.8 (Stable - IPv4 & IPv6) 2 July 2021
#### Features
* Dual stack: client can be shaped by same qdisc for both IPv4 and IPv6
* Up to 1000 clients (IPv4/IPv6)
* Real world asymmetrical throughput: between 2Gbps and 4.5Gbps depending on CPU single thread performance. 
* HTB+fq_codel or HTB+cake
* Shape Clients by Access Point / Node capacity
* TC filters split into groups through hashing filters to increase throughput
* Simple client management via csv file
* Simple statistics - table shows top 20 subscribers by packet loss, with APs listed

#### Limitations
* Qdisc locking problem limits throughput of HTB used in v0.8 (solved in v0.9). Tested up to 4Gbps/500Mbps asymmetrical throughput using [Microsoft Ethr](https://github.com/microsoft/ethr) with n=500 streams. High quantities of small packets will reduce max throughput in practice.
* Linux tc hash tables can only handle [~4000 rules each.](https://stackoverflow.com/questions/21454155/linux-tc-u32-filters-strange-error) This limits total possible clients to 1000 in v0.8.

### v0.9 (Stable - IPv4) 11 Jul 2021
#### Features
* [XDP-CPUMAP-TC](https://github.com/xdp-project/xdp-cpumap-tc) integration greatly improves throughput, allows many more IPv4 clients, and lowers CPU use. Latency reduced by half on networks previously limited by single-CPU / TC QDisc locking problem in v.0.8.
* Tested up to 10Gbps asymmetrical throughput on dedicated server (lab only had 10G router). v0.9 is estimated to be capable of an asymmetrical throughput of 20Gbps-40Gbps on a dedicated server with 12+ cores.
* ![Throughput](docs/10Gbps.png?raw=true "Throughput")
* MQ+HTB+fq_codel or MQ+HTB+cake
* Now defaults to 'cake diffserv4' for optimal client performance
* Client limit raised from 1,000 to 32,767
* Shape Clients by Access Point / Node capacity
* APs equally distributed among CPUs / NIC queues to greatly increase throughput
* Simple client management via csv file
#### Considerations
* Each Node / Access Point is tied to a queue and CPU core. Access Points are evenly distributed across CPUs. Since each CPU can usually only accomodate up to 4Gbps, ensure any single Node / Access Point will not require more than 4Gbps throughput.
#### Limitations
* Not dual stack, clients can only be shaped by IPv4 address for now in v0.9. Once IPv6 support is added to [XDP-CPUMAP-TC](https://github.com/xdp-project/xdp-cpumap-tc) we can then shape IPv6 as well.
* XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).

### v1.0 (Stable - IPv4) 11 Dec 2021
#### Features
* Can now shape by Site, in addition to by AP and by Client
#### Considerations
* If you shape by Site, each site is tied to a queue and CPU core. Sites are evenly distributed across CPUs. Since each CPU can usually only accomodate up to 4Gbps, ensure any single Site will not require more than 4Gbps throughput.
* If you shape by Acess Point, each Access Point is tied to a queue and CPU core. Access Points are evenly distributed across CPUs. Since each CPU can usually only accomodate up to 4Gbps, ensure any single Access Point will not require more than 4Gbps throughput.
#### Limitations
* As with 0.9, not yet dual stack, clients can only be shaped by IPv4 address until IPv6 support is added to [XDP-CPUMAP-TC](https://github.com/xdp-project/xdp-cpumap-tc). Once that happens we can then shape IPv6 as well.
* XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).

### v1.1 (Alpha - IPv4) 2022
![Screenshot](docs/v1.1-alpha-preview.jpg?raw=true "Screenshot")
#### Features
* Tested up to 11Gbps asymmetrical throughput in real world deployment with 5000+ clients.
* Network heirarchy can be mapped to the network.json file. This allows for both simple network heirarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client).
* Graphing of bandwidth to InfluxDB. Parses bandwidth data from "tc -s qdisc show" command, minimizing CPU use.
* Graphing of TCP latency to InfluxDB - via [PPing](https://github.com/pollere/pping) integration.
#### Considerations
* Any top-level parent node is tied to a single CPU core. Top-level nodes are evenly distributed across CPUs. Since each CPU can usually only accomodate up to 4Gbps, ensure any single top-level parent node will not require more than 4Gbps throughput.
#### Limitations
* As with 0.9 and v1.0, not yet dual stack, clients can only be shaped by IPv4 address until IPv6 support is added to [XDP-CPUMAP-TC](https://github.com/xdp-project/xdp-cpumap-tc). Once that happens we can then shape IPv6 as well.
* XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).

## NMS/CRM Integrations
### UISP Integration
There is a rudimentary UISP integration included in v1.1-alpha.
Instead, you may want to use the [RUST-based UISP integration](https://github.com/thebracket/libre_qos_rs/tree/main/uisp_integration) developed by [@thebracket](https://github.com/thebracket/) for v1.1 and above.
[@thebracket](https://github.com/thebracket/) was kind enough to produce this great tool, which maps the actual network heirarchy to the network.json and Shaper.csv formats LibreQoS can use.

## General Requirements
* VM or physical server. Physical server will perform better and better utilize all CPU cores.
* One management network interface, completely seperate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* Dedicated Network Interface Card 
  * NIC must have two or more interfaces for traffic shaping.
  * NIC must have multiple TX/RX transmit queues. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * Known supported cards:
    * [NVIDIA ConnectX-4 MCX4121A-XCAT](https://store.mellanox.com/products/nvidia-mcx4121a-xcat-connectx-4-lx-en-adapter-card-10gbe-dual-port-sfp28-pcie3-0-x8-rohs-r6.html)
    * [Intel X710](https://www.fs.com/products/75600.html)
    * Intel X520
* Ubuntu Server 21.10 or above recommended. All guides assume Ubuntu Server 21.10. Ubuntu Desktop is not recommended as it uses NetworkManager instead of Netplan.
* v0.9+: Requires kernel version 5.9 or above for physical servers, and kernel version 5.14 or above for VM.
* v0.8: Requires kernel version 5.1 or above.
* Python 3, PIP, and some modules (listed in respective guides).
* Choose a CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html) within your budget. Generally speaking, any new CPU above $200 can probably handle shaping up to 2Gbps.
  * Recommendations for 10G throughput:
    * [AMD Ryzen 9 5900X](https://www.bestbuy.com/site/amd-ryzen-9-5900x-4th-gen-12-core-24-threads-unlocked-desktop-processor-without-cooler/6438942.p?skuId=6438942)
    * [Intel Core i7-12700KF](https://www.bestbuy.com/site/intel-core-i7-12700kf-desktop-processor-12-8p-4e-cores-up-to-5-0-ghz-unlocked-lga1700-600-series-chipset-125w/6483674.p?skuId=6483674)

## Installation and Usage Guide

Best Performance, IPv4 Only:
📄 (alpha) [LibreQoS v1.1 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.1-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-21.10)

📄 [LibreQoS v1.0 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.0-Installation-&-Usage-Guide---Physical-Server-and-Ubuntu-21.10)

📄 [LibreQoS v0.9 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.9-Installation-&-Usage-Guide----Physical-Server-and-Ubuntu-21.10)

Good Performance, IPv4 Only:

📄 [LibreQoS v0.9 Installation & Usage Guide Proxmox and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.9-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-21.10)

OK Performance, IPv4 and IPv6:

📄 [LibreQoS 0.8 Installation and Usage Guide - Proxmox and Ubuntu 20.04 LTS](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v0.8-Installation-&-Usage-Guide----Proxmox-and-Ubuntu-20.04)

## Donate

LibreQoS itself is Open-Source/GPL software: there is no cost to use it.

LibreQoS makes great use of fq\_codel and CAKE - two open source AQMs whose development is led by Dave Täht, and contributed to by dozens of others from around the world. Without Dave's work and advocacy, there would be no LibreQoS, Preseem, or Paraqum. 

If LibreQoS helps your network, please [contribute to Dave's Patreon.](https://www.patreon.com/dtaht) Donating just $0.2/sub/month ($100/month for 500 subs) comes out to be 60% less than any proprietary solution, plus you get to ensure the continued development and improvement of CAKE. Dave's work has been essential to improving internet connectivity around the world. Let's all pitch in to help his mission.

<a href="https://www.patreon.com/dtaht">
  <img src="https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/donate.png" alt="Donate" width="289" />
</a>

## Special Thanks
Special thanks to Dave Täht, Jesper Dangaard Brouer, Toke Høiland-Jørgensen, Kumar Kartikeya Dwivedi, Kathleen M. Nichols, Maxim Mikityanskiy, Yossi Kuperman, and Rony Efraim for their many contributions to the Linux networking stack. Thank you Phil Sutter, Bert Hubert, Gregory Maxwell, Remco van Mook, Martijn van Oosterhout, Paul B Schroeder, and Jasper Spaans for contributing to the guides and documentation listed below. Thanks to Leo Manuel Magpayo for his help improving documentation and for testing. Thanks to everyone on the [Bufferbloat mailing list](https://lists.bufferbloat.net/listinfo/) for your help and contibutions.

# Made possible by
* [fq_codel and CAKE](https://www.bufferbloat.net/projects/)
* [xdp-cpumap-tc](https://github.com/xdp-project/xdp-cpumap-tc)
* [PPing](https://github.com/pollere/pping)
* [LibTins](http://libtins.github.io/)
* [InfluxDB](https://github.com/influxdata/influxdb)

# Other References
* https://tldp.org/HOWTO/Adv-Routing-HOWTO/lartc.adv-filter.hashing.html
* http://linux-ip.net/gl/tc-filters/tc-filters.html
* http://linux-tc-notes.sourceforge.net/tc/doc/cls_u32.txt
* https://netdevconf.info/0x14/pub/papers/44/0x14-paper44-talk-paper.pdf

# License
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
