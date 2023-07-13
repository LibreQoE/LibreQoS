## Network Interface Requirements
* One management network interface completely separate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* Dedicated Network Interface Card for Shaping Interfaces
  * NIC must have 2 or more interfaces for traffic shaping.
  * NIC must have multiple TX/RX transmit queues. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * Known supported cards:
    * [NVIDIA Mellanox MCX512A-ACAT](https://www.fs.com/products/119649.html)
    * NVIDIA Mellanox MCX416A-CCAT
    * [Intel X710](https://www.fs.com/products/75600.html)
    * Intel X520
