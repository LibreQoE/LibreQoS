## System Requirements
### Physical server
* LibreQoS requires a dedicated, physical x86_64 device.
* While it is technically possible to run LibreQoS in  VM, it is not officially supported, and comes at a significant 30% performance penalty (even when using NIC passthrough). For VMs, NIC passthrough is required for throughput above 1 Gbps (XDP vs generic XDP).

### CPU
* 2 or more CPU cores
* A CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html#server-thread) within your budget. Queuing is very CPU-intensive, and requires high single-thread performance.

Single-thread CPU performance will determine the maximum capacity of a single HTB (cpu core), and in turn, the maximum capacity of any top level node in the network hierarchy (for example, top-level sites in your network). This also impacts the maximum speed plan you can offer customers within safe margins.

| Top Level Node Max  | Single-Thread Score      |
| --------------------| ------------------------ |
| 1 Gbps              | 1000                     |
| 2 Gbps              | 1500                     |
| 3 Gbps              | 2000                     |
| 5 Gbps              | 4000                     |

| Customer Max Plan   | Single-Thread Score      |
| --------------------| ------------------------ |
| 100 Mbps            | 1000                     |
| 250 Mbps            | 1250                     |
| 500 Mbps            | 1500                     |
| 1 Gbps              | 1750                     |
| 2.5 Gbps            | 2000                     |
| 5 Gbps              | 4000                     |

Below is a table of approximate aggregate capacity, assuming a a CPU with a [single thread](https://www.cpubenchmark.net/singleThread.html#server-thread) performance of 1000 / 2000 / 4000:

| CPU Cores | Single-Thread Score = 1000 | Single-Thread Score = 2000 | Single-Thread Score = 4000 |
|-----------|----------------------------|----------------------------|----------------------------|
| 2         | 1 Gbps                     | 3 Gbps                     | 7 Gbps                     |
| 4         | 3 Gbps                     | 5 Gbps                     | 13 Gbps                    |
| 6         | 4 Gbps                     | 8 Gbps                     | 20 Gbps                    |
| 8         | 5 Gbps                     | 10 Gbps                    | 27 Gbps                    |
| 16        | 10 Gbps                    | 21 Gbps                    | 54 Gbps                    |
| 32        | 21 Gbps                    | 42 Gbps                    | 108 Gbps                   |
| 64        | 42 Gbps                    | 83 Gbps                    | 216 Gbps                   |

### Memory
* Recommended RAM:

| RAM (using CAKE) | Max Subscribers |
| ---------------- | --------------- |
| 8 GB             | 1,000           |
| 16 GB            | 2,500           | 
| 32 GB            | 5,000           |
| 64 GB            | 10,000          |
| 128 GB           | 25,000          |
| 256 GB           | 50,000          |

### Server Recommendations
Here are some convenient, off-the-shelf server options to consider:
| Throughput | Per Node / Per CPU Core| Model | CPU Option | RAM Option | NIC Option | Extras | Temp Range |
| --- | --- | --- | --- | --- | --- | --- | --- | 
| 2.5 Gbps | 1 Gbps | [Supermicro SYS-E102-13R-E](https://store.supermicro.com/us_en/compact-embedded-iot-i5-1350pe-sys-e102-13r-e.html) | Default | 2x8GB | Built-in | [USB-C RJ45](https://www.amazon.com/Anker-Ethernet-PowerExpand-Aluminum-Portable/dp/B08CK9X9Z8/)| 0°C ~ 40°C (32°F ~ 104°F) |
| 10 Gbps | 3 Gbps | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | 8124P | 2x16GB | Mellanox (2 x SFP28) | | 0°C ~ 40°C (32°F ~ 104°F) |
| 10 Gbps | 5 Gbps | [Supermicro SYS-511R-M](https://store.supermicro.com/us_en/mainstream-1u-sys-511r-m.html) | E-2488 | 2x32GB | 10-Gigabit X710-BM2 (2 x SFP+) | | 0°C ~ 40°C (32°F ~ 104°F) |
| 10 Gbps | 5 Gbps | [Dell PowerEdge R260](https://www.dell.com/en-us/shop/dell-poweredge-servers/new-poweredge-r260-rack-server/spd/poweredge-r260/pe_r260_tm_vi_vp_sb?configurationid=2cd33e43-57a3-4f82-aa72-9d5f45c9e24c) | E-2456 | 2x32GB | Intel X710-T2L (2 x 10G RJ45) | | 5–40°C (41–104°F) |
| 25 Gbps |  3 Gbps | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | 8534P | 4x16GB | Mellanox (2 x SFP28) | | 0°C ~ 40°C (32°F ~ 104°F) |

### Network Interface Requirements
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
