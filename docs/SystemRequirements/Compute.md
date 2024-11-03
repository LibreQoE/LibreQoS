## System Requirements
### Physical server
* LibreQoS requires a dedicated, physical x86_64 device.
* While it is technically possible to run LibreQoS in  VM, it is not officially supported, and comes at a significant 30% performance penalty (even when using NIC passthrough). For VMs, NIC passthrough is required for throughput above 1 Gbps (XDP vs generic XDP).

### CPU
* 2 or more CPU cores
* A CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html#server-thread) within your budget. Queuing is very CPU-intensive, and requires high single-thread performance.

Single-thread CPU performance will determine the max throughput of a single HTB (cpu core), and in turn, what max speed plan you can offer customers.

| Customer Max Plan   | Passmark Single-Thread   |
| --------------------| ------------------------ |
| 100 Mbps            | 1000                     |
| 250 Mbps            | 1250                     |
| 500 Mbps            | 1500                     |
| 1 Gbps              | 2000                     |
| 2.5 Gbps            | 3000                     |
| 5 Gbps              | 4000                     |

Below is a table of approximate aggregate throughput capacity, assuming a a CPU with a [single thread](https://www.cpubenchmark.net/singleThread.html#server-thread) performance of 2700 / 4000:

| Aggregate Throughput    | CPU Cores Needed (>2700 single-thread) | CPU Cores Needed (>4000 single-thread) |
| ------------------------| -------------------------------------- | -------------------------------------- |
| 500 Mbps                | 2                                      | 2                                      |
| 1 Gbps                  | 4                                      | 2                                      |
| 5 Gbps                  | 6                                      | 4                                      |
| 10 Gbps                 | 8                                      | 6                                      |
| 20 Gbps                 | 16                                     | 8                                      |
| 50 Gbps                 | 32                                     | 16                                     |
| 100 Gbps                | 64                                     | 32                                     |

So for example, an ISP delivering 1Gbps service plans with 10Gbps aggregate throughput would choose a CPU with a 2500+ single-thread score and 8 cores, such as the Intel Xeon E-2388G @ 3.20GHz.

### Memory
* Recommended RAM:

| Subscribers   | RAM           |
| ------------- | ------------- |
| 100           | 8 GB          |
| 1,000         | 16 GB         |
| 5,000         | 64 GB         |
| 10,000        | 128 GB        |
| 20,000        | 256 GB        |

### Server Recommendations
Here are some convenient, off-the-shelf server options to consider:
| Throughput   | Model | CPU Option | RAM Option | NIC Option | Extras | Temp Range |
| --- | --- | --- | --- | --- | --- | --- | 
| 2.5 Gbps | [Supermicro SYS-E102-13R-E](https://store.supermicro.com/us_en/compact-embedded-iot-i5-1350pe-sys-e102-13r-e.html) | Default | 2x8GB | Built-in | [USB-C RJ45](https://www.amazon.com/Anker-Ethernet-PowerExpand-Aluminum-Portable/dp/B08CK9X9Z8/)| 0°C ~ 40°C (32°F ~ 104°F) |
| 10 Gbps | [Supermicro AS -1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | 8124P | 2x16GB | Mellanox (2 x SFP28) | | 0°C ~ 40°C (32°F ~ 104°F) |
| 25 Gbps | [Supermicro AS -1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | 8534P | 4x16GB | Mellanox (2 x SFP28) | | 0°C ~ 40°C (32°F ~ 104°F) |

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
