# LibreQoS

## Requirements
* Recent Linux kernel
* tc (available via package iproute2)
* Cake

    git clone https://github.com/dtaht/sch_cake.git

    cd sch_cake
    make; sudo make install

## Features
* Cake
* fq_codel
* tc filters divided into hash tables to significantly increase efficiency

## Server requirements
* 8GB RAM or more recommended

## License
Copyright (C) 2020 Robert Chacon

LibreQoS is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 2 of the License, or
(at your option) any later version.

Foobar is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with LibreQoS.  If not, see <http://www.gnu.org/licenses/>.
