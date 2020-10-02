# LibreQoS
A simple way to shape hundreds of clients and reduce bufferbloat using cake or fq_codel. This is alpha software, please do not deploy in production.

## Lab Requirements
* Edge and Core routers with MTU 1500 on links between them
* OSPF primary link (low cost) through the server running LibreQoS
* OSPF backup link recommended

## Server Requirements
* VM or physical server
    * One management network interface
    * Two dedicated network interface cards, preferably SFP+ capable
* 8GB RAM or more recommended
* Python 3
* Recent Linux kernel
* tc (available via package iproute2)
* Cake

    git clone https://github.com/dtaht/sch_cake.git

    cd sch_cake
    make; sudo make install

## Features
* Cake (Common Applications Kept Enhanced)
* fq_codel
* HTB (Hierarchy Token Bucket)
* tc filters divided into groups with hashing filters to significantly increase efficiency

## How to use
* Add linux interface bridge br0 to the two dedicated interfaces
* Modify setting parameters in LibreQoS.py to suit your environment
* Run:
sudo python3 ./LibreQoS.py

## Special Thanks
Thank you to the hundreds of contributors to the cake and fq_codel projects.

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
