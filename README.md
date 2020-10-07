# LibreQoS
LibreQoS is a python application that allows you to apply fq_codel traffic shaping on hundreds of customers. <a href="https://www.bufferbloat.net/projects/codel/wiki/">Fq_codel</a> is a Free and Open Source Active Queue Management algorithm that reduces bufferbloat, and can improve the quality of customer connections significantly. LibreQoS features the ability to import devices from LibreNMS and UNMS at runtime using API calls. It then apples hundreds of filter rules to direct customer traffic through individual fq_codel instances within an <a href="https://linux.die.net/man/8/tc-htb">HTB</a> (HTB+fq_codel). By utilizing <a href="https://tldp.org/HOWTO/Adv-Routing-HOWTO/lartc.adv-filter.hashing.html">hashing filters</a>, thousands of rules can be applied with minimal impact on traffic throughput or CPU use. This is alpha software, please do not deploy in production without thorough testing. If you need a stable paid commercial alternative, please check out <a href="https://www.preseem.com/">Preseem</a>, which has great metrics tools and integration with many CRM and NMS platforms.

## Features
* HTB + fq_codel
* Experimental support for CAKE (Common Applications Kept Enhanced)
* TC filters divided into groups with hashing filters to significantly increase efficiency and minimize resource use
* High-efficiency
* Basic statistics (Top 10 CPEs experiencing packet loss)

## Integration
* LibreNMS device import
* UNMS/UCRM device import

## Requirements
* Edge and Core routers with MTU 1500 on links between them
   * If you use MPLS, you would terminate MPLS traffic at the core router
* OSPF primary link (low cost) through the server running LibreQoS
* OSPF backup link
![Diagram](docs/diagram.png?raw=true "Diagram")

## Server Requirements
* VM or physical server
* One management network interface
* Two dedicated network interface cards, preferably SFP+ capable
* Python 3
* Recent Linux kernel
* recent tc-fq_codel provided by package iproute2

## Known issues
* Linux TC filters may be left in the memory cache after being removed/disassociated with qdiscs. However, that memory space tends to get overwritten as long as the IP scheme of your network isn't changing constantly somehow. If after a few months you need to clear memory cache, use
```
sudo sh -c 'echo 1 >/proc/sys/vm/drop_caches'
```
On <a href="https://www.reddit.com/r/Proxmox/comments/asakcb/problem_with_ram_cache/">ProxMox VMs</a> you need to do some tweaks to allow that freed up memory to be reflected on the hypervisor. 
## Adding the bridge between in/out interface NICs
* Add linux interface bridge br0 to the two dedicated interfaces
    * For example on Ubuntu Server 20.04 which uses NetPlan, you would add the following to the .yaml file in /etc/netplan/
```
bridges:
    br0:
      interfaces:
           - eth4
           - eth5
```
## Run LibreQoS
* Modify setting parameters in ispConfig.py to suit your environment
* Run:
```
sudo python3 ./LibreQoS.py
```
## Statistics
Basic statistics are now available by running
```
python3 ./stats.py
```
after successful execution of ./LibreQoS.py or ./scheduled.py
## Running as a service
You can use the scheduled.py file to set the time of day you want the shapers to be refreshed at after the initial run.
On linux distributions that use systemd, such as Ubuntu, add the following to /etc/systemd/system/LibreQoS.service
```
[Unit]
After=network.service

[Service]
WorkingDirectory=/home/$USER/LibreQoSDirectory
ExecStart=/usr/bin/python3 /home/$USER/LibreQoSDirectory/scheduled.py
Restart=always

[Install]
WantedBy=default.target
```
Then run
```
sudo systemctl start LibreQoS.service
```
## Real World Performance
This customer is using a Ubiquiti LTU-LR CPE with QoS shaping applied at 215Mbps down and 30Mbps up.

<img src="docs/customerExample.jpg" width="500">

## Server Spec Recommendations
* For up to 1Gbps
    * 4+ CPU cores
    * 4GB RAM
    * 32GB Disk Space
    * Passmark score of 13,000 or more (AMD Ryzen 5 2600 or better)
* For up to 2Gbps
    * 6+ CPU cores
    * 6GB RAM
    * 32GB Disk Space
    * Passmark score of 17,000 or more (AMD Ryzen 5 3600 or better)
* For up to 5Gbps
    * 8+ CPU cores
    * 8GB RAM
    * 32GB Disk Space
    * Passmark score of 23,000 or more (AMD Ryzen 7 3800X or better)
* For up to 10Gbps
    * 16+ CPU cores
    * 16GB RAM
    * 32GB Disk Space
    * Passmark score of 38,000 or more (AMD Ryzen 9 3950X or better)

https://www.cpubenchmark.net/high_end_cpus.html

## Special Thanks
Thank you to the hundreds of contributors to the cake and fq_codel projects. Thank you to Phil Sutter, Bert Hubert, Gregory Maxwell, Remco van Mook, Martijn van Oosterhout, Paul B Schroeder, and Jasper Spaans for contributing to the guides and documentation listed below.

## References
* https://tldp.org/HOWTO/Adv-Routing-HOWTO/lartc.adv-filter.hashing.html
* http://linux-ip.net/gl/tc-filters/tc-filters.html

## License
Copyright (C) 2020 Robert Chacón

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
