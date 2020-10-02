# LibreQoS
A simple way to shape hundreds of clients and reduce bufferbloat using cake or fq_codel.

## Requirements
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

## Server requirements
* 8GB RAM or more recommended

## How to use
* Modify setting parameters in LibreQoS.py to suit your environment
* Run:
sudo python3 ./LibreQoS.py

## Special Thanks
Thank you to the hundreds of contributors to the cake and fq_codel projects.

## License
Copyright (C) 2020 Robert Chacon

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
