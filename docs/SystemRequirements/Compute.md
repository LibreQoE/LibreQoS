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
| 16 GB            | 2,000           | 
| 32 GB            | 5,000           |
| 64 GB            | 10,000          |
| 128 GB           | 20,000          |
| 256 GB           | 40,000          |

### Server Recommendations
Here are some convenient, off-the-shelf server options to consider:

| **Throughput**              | 2.5 Gbps                                                                                                           | 10 Gbps                                                                                                             | 10 Gbps                                                                                   | 10 Gbps                                                                                                                                                                                                  | 25 Gbps                                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **Per Node / Per CPU Core** | 1 Gbps                                                                                                             | 3 Gbps                                                                                                              | 5 Gbps                                                                                    | 5 Gbps                                                                                                                                                                                                   | 3 Gbps                                                                                                              |
| **Model**                   | [Supermicro SYS-E102-13R-E](https://store.supermicro.com/us_en/compact-embedded-iot-i5-1350pe-sys-e102-13r-e.html) | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | [Supermicro SYS-511R-M](https://store.supermicro.com/us_en/mainstream-1u-sys-511r-m.html) | [Dell PowerEdge R260](https://www.dell.com/en-us/shop/dell-poweredge-servers/new-poweredge-r260-rack-server/spd/poweredge-r260/pe_r260_tm_vi_vp_sb?configurationid=2cd33e43-57a3-4f82-aa72-9d5f45c9e24c) | [Supermicro AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) |
| **CPU Option**              | Default                                                                                                            | 8124P                                                                                                               | E-2488                                                                                    | E-2456                                                                                                                                                                                                   | 8534P                                                                                                               |
| **RAM Option**              | 2x8GB                                                                                                              | 2x16GB                                                                                                              | 2x32GB                                                                                    | 2x32GB                                                                                                                                                                                                   | 4x16GB                                                                                                              |
| **NIC Option**              | Built-in                                                                                                           | Mellanox (2 x SFP28)                                                                                                | 10-Gigabit X710-BM2 (2 x SFP+)                                                            | Intel X710-T2L (2 x 10G RJ45)                                                                                                                                                                            | Mellanox (2 x SFP28)                                                                                                |
| **Extras**                  | [USB-C RJ45](https://www.amazon.com/Anker-Ethernet-PowerExpand-Aluminum-Portable/dp/B08CK9X9Z8/)                   |                                                                                                                     |                                                                                           |                                                                                                                                                                                                          |                                                                                                                     |
| **Temp Range**              | 0°C ~ 40°C                                                                                                         | 0°C ~ 40°C                                                                                                          | 0°C ~ 40°C                                                                                | 5–40°C                                                                                                                                                                                                   | 0°C ~ 40°C                                                                                                          |
| **Temp Range**              | (32°F ~ 104°F)                                                                                                     | (32°F ~ 104°F)                                                                                                      | (32°F ~ 104°F)                                                                            | (41–104°F)                                                                                                                                                                                               | (32°F ~ 104°F)                                                                                                      |

### Network Interface Requirements
* One management network interface completely separate from the traffic shaping interfaces. Usually this would be the Ethernet interface built in to the motherboard.
* A Dedicated Network Interface Card for Two Shaping Interfaces

Officially supported Network Interface Cards for the two shaping interfaces are listed below:

| NIC Controller         | Port Speed       | Suggested Models                                                                        | Known Issues                                                                                  |
|------------------------|------------------|-----------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| Intel X520             | 10 Gbps          |                                                                                         | Module compatibility*                                                                         |
| Intel X710             | 10 Gbps          | [X710-BM2 10G]( https://www.fs.com/products/75600.html?now_cid=4253)                    | Module compatibility*                                                                         |
| Intel XXV710           | 10 / 25 Gbps     | [XXV710 25G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896)         | Module compatibility*                                                                         |
| Intel XL710            | 10 / 40 Gbps     | [XL710-BM2 40G](https://www.fs.com/products/75604.html?attribute=67774&id=1709896 )     | Module compatibility*                                                                         |
| Mellanox ConnectX-4 Lx | 10/25/40/50 Gbps |                                                                                         | No known issues.                                                                              |
| Mellanox ConnectX-5    | 100 Gbps         | [MCX516A-CCAT 100G](https://www.fs.com/products/119647.html?attribute=67743&id=3746410) | Extreme heat at high load (50+ Gbps). Use Liquid CPU Cooler kit on chip to avoid overheating. |
| Mellanox ConnectX-6    | 10/25 Gbps       | [MCX631102AN-ADAT](https://www.fs.com/products/212177.html?now_cid=4014)                | No known issues.                                                                              |
| Mellanox ConnectX-6    | 100 Gbps         | [MCX623106AN-CDAT 100G](https://www.fs.com/products/119646.html?now_cid=4014)           | No known issues.                                                                              |
| Mellanox ConnectX-7    | 200 Gbps         | [MCX755106AS-HEAT 200G](https://www.fs.com/products/242589.html?now_cid=4014)           | No known issues.                                                                              |

(*) Intel often vendor-locks SFP+ module compatibility. Check module compatibility before buying. Mellanox does not have this problem.

**We will ONLY provide support for systems using a NIC listed above**. Some other NICs *may* work, but will not be officially supported by LibreQoS. If you want to *test* the compatability of another card, please be aware of these fundamental NIC requirements:
  * NIC must have multiple TX/RX transmit queues, greater than or equal to the number of CPU cores. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * NIC must have [XDP driver support](https://github.com/xdp-project/xdp-project/blob/master/areas/drivers/README.org) for high-throughput (10 Gbps+).

If you discover that a card not listed in the table above is compatible, please let us know by emailing support [at] libreqos.io.
