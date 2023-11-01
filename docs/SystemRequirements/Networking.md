## Network Interface Requirements
* One management network interface completely separate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* Dedicated Network Interface Card for Shaping Interfaces
  * NIC must have 2 or more interfaces for traffic shaping.
  * NIC must have multiple TX/RX transmit queues, greater than or equal to the number of CPU cores. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * NIC must have [XDP driver support](https://github.com/xdp-project/xdp-project/blob/master/areas/drivers/README.org)
  * Supported cards:
    * Intel X520
    * Intel X550
    * [Intel X710](https://www.fs.com/products/75600.html)
    * Intel XL710
    * Intel XXV710
    * NVIDIA Mellanox ConnectX-4 series
    * [NVIDIA Mellanox ConnectX-5 series](https://www.fs.com/products/119649.html)
    * NVIDIA Mellanox ConnectX-6 series
    * NVIDIA Mellanox ConnectX-7 series
  * Unsupported cards:
    * Broadcom (all)
    * NVIDIA Mellanox ConnectX-3 series
    * Intel E810
    * We will not provide support for any system using an unsupported NIC
