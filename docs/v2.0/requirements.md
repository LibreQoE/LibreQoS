# System Requirements

LibreQoS can be run either on a dedicated physical server (bare metal) or as a VM. Ubuntu Server 24.04 is the supported operating system.

## Physical Server (Bare Metal)

### CPU
* 2 or more CPU cores are required
* Choose a CPU with high [single-thread performance](https://www.cpubenchmark.net/singleThread.html#server-thread) within your budget. Queuing is CPU-intensive, and requires high single-thread performance.

Single-thread CPU performance determines the maximum capacity of a single HTB (CPU core). This, in turn, affects the maximum capacity of any top-level node in the network hierarchy (e.g., top-level sites in your network). This also impacts the maximum speed plan you can offer customers within safe margins.

| Single-Thread Score | Top-Level Node Max | Customer Plan Max |
|:-------------------:|:------------------:|:-----------------:|
| 1000                | 1 Gbps             | 100 Mbps          |
| 1500                | 2 Gbps             | 500 Mbps          |
| 2000                | 3 Gbps             | 1 Gbps            |
| 3000                | 4 Gbps             | 2 Gbps            |
| 4000                | 5 Gbps             | 3 Gbps            |

Below is a table of approximate aggregate capacity, assuming a CPU with a [single thread](https://www.cpubenchmark.net/singleThread.html#server-thread) performance rating of 1000, 2000, 3000, or 4000:

| CPU Cores | Single-Thread Score: 1000 | Single-Thread Score: 2000 | Single-Thread Score: 3000 | Single-Thread Score: 4000 |
|:---------:|:-------------------------:|:-------------------------:|:-------------------------:|:-------------------------:|
| 2         | 1 Gbps                    | 3 Gbps                    | 5 Gbps                    | 7 Gbps                    |
| 4         | 3 Gbps                    | 5 Gbps                    | 9 Gbps                    | 13 Gbps                   |
| 6         | 4 Gbps                    | 8 Gbps                    | 14 Gbps                   | 20 Gbps                   |
| 8         | 5 Gbps                    | 10 Gbps                   | 18 Gbps                   | 27 Gbps                   |
| 16        | 10 Gbps                   | 21 Gbps                   | 36 Gbps                   | 54 Gbps                   |
| 32        | 21 Gbps                   | 42 Gbps                   | 72 Gbps                   | 108 Gbps                  |
| 64        | 42 Gbps                   | 84 Gbps                   | 144 Gbps                  | 216 Gbps                  |
| 128       | 84 Gbps                   | 168 Gbps                  | 288 Gbps                  |                           |

### Hyper-threading

It is recommended to disable Hyper-Threading (Simultaneous Multi-Threading) in the BIOS/UEFI settings, as it can interfere with XDP processing.

### Memory
* Recommended RAM:

| RAM (using CAKE) | Max Subscribers |
| ---------------- | --------------- |
| 4 GB             | 1,000           |
| 8 GB             | 2,000           |
| 16 GB            | 5,000           | 
| 32 GB            | 10,000          |
| 64 GB            | 20,000          |
| 128 GB           | 40,000          |

### Disk Space

50 GB of disk space or more is generally recommended, both for servers and VM deployments.

### Device Recommendations
#### Small Form Factor (1G to 10G)

|        Throughput       |                                         10 Gbps                                        |
|:-----------------------:|:--------------------------------------------------------------------------------------:|
| Per Node / Per CPU Core | 5 Gbps                                                                                 |
| Manufacturer            | Minisforum                                                                             |
| Model                   | [MS-01](https://store.minisforum.com/products/minisforum-ms-01?variant=46174128898293) |
| CPU Option              | i9-12900H                                                                              |
| RAM Option              | 1x32GB                                                                                 |
| NIC Option              | Built-in                                                                               |
| Temp Range              | 0°C ~ 40°C                                                                             |
| Temp Range              | (32°F ~ 104°F)                                                                         |
| ECC                     | No                                                                                     |
| Power                   | 19V DC                                                                                 |

#### Rackmount Servers (10G to 100G)

|        Throughput       |                                     10 Gbps                                    |                                                                                               10 Gbps                                                                                               |                                                  25 Gbps                                                 | 50 Gbps                                                                               | 100 Gbps                                                                            |
|:-----------------------:|:------------------------------------------------------------------------------:|:---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------:|:--------------------------------------------------------------------------------------------------------:|---------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------|
| Per Node / Per CPU Core | 5 Gbps                                                                         | 5 Gbps                                                                                                                                                                                              | 3 Gbps                                                                                                   | 3 Gbps                                                                                | 4 Gbps                                                                              |
| Manufacturer            | Supermicro                                                                     | Dell                                                                                                                                                                                                | Supermicro                                                                                               | Supermicro                                                                            | Supermicro                                                                          |
| Model                   | [SYS-511R-M](https://store.supermicro.com/us_en/mainstream-1u-sys-511r-m.html) | [PowerEdge R260](https://www.dell.com/en-us/shop/dell-poweredge-servers/new-poweredge-r260-rack-server/spd/poweredge-r260/pe_r260_tm_vi_vp_sb?configurationid=2cd33e43-57a3-4f82-aa72-9d5f45c9e24c) | [AS-1115S-FWTRT](https://store.supermicro.com/us_en/1u-amd-epyc-8004-compact-server-as-1115s-fwtrt.html) | [AS-1015SV-WTNRT](https://store.supermicro.com/us_en/1u-amd-wio-as-1015sv-wtnrt.html) | [AS -2015CS-TNR](https://store.supermicro.com/us_en/clouddc-amd-as-2015cs-tnr.html) |
| CPU Option              | E-2488                                                                         | E-2456                                                                                                                                                                                              | 8534P                                                                                                    | 8534P                                                                                 | 9745                                                                                |
| RAM Option              | 1x32GB                                                                         | 1x32GB                                                                                                                                                                                              | 4x16GB                                                                                                   | 2x64GB                                                                                | 4x64GB                                                                              |
| NIC Option              | 10-Gigabit X710-BM2 (2 x SFP+)                                                 | Intel X710-T2L (2 x 10G RJ45)                                                                                                                                                                       | Mellanox (2 x SFP28)                                                                                     | Mellanox 100-Gigabit (2 x QSFP56)                                                     | MCX653106A-HDAT                                                                     |
| Temp Range              | 0°C ~ 40°C                                                                     | 5–40°C                                                                                                                                                                                              | 0°C ~ 40°C                                                                                               | 0°C ~ 40°C                                                                            | 0°C ~ 40°C                                                                          |
| Temp Range              | (32°F ~ 104°F)                                                                 | (41–104°F)                                                                                                                                                                                          | (32°F ~ 104°F)                                                                                           | (32°F ~ 104°F)                                                                        | (32°F ~ 104°F)                                                                      |
| ECC                     | Yes                                                                            | Yes                                                                                                                                                                                                 | Yes                                                                                                      | Yes                                                                                   | Yes                                                                                 |
| Power                   | AC                                                                             | AC                                                                                                                                                                                                  | AC                                                                                                       | AC                                                                                    | AC                                                                                  |

Another cost-effective solution is to procure a used server from a reputable vendor, such as [TheServerStore](https://www.theserverstore.com/).
Such vendors often stock servers capable of 10 Gbps throughput, for around $500 USD.

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
| Mellanox ConnectX-6    | 10/25 Gbps       | [MCX631102AN-ADAT](https://www.fs.com/products/212177.html?now_cid=4014)                | No known issues.                                                                              |
| Mellanox ConnectX-6    | 100 Gbps         | [MCX623106AN-CDAT 100G](https://www.fs.com/products/119646.html?now_cid=4014)           | No known issues.                                                                              |
| Mellanox ConnectX-7    | 200 Gbps         | [MCX755106AS-HEAT 200G](https://www.fs.com/products/242589.html?now_cid=4014)           | No known issues.                                                                              |

(*) Intel often vendor-locks SFP+ module compatibility. Check module compatibility before buying. Mellanox does not have this problem.

**We will ONLY provide support for systems using a NIC listed above**. Some other NICs *may* work, but will not be officially supported by LibreQoS. If you want to *test* the compatability of another card, please be aware of these fundamental NIC requirements:
  * NIC must have multiple TX/RX transmit queues, greater than or equal to the number of CPU cores. [Here's how to check from the command line](https://serverfault.com/questions/772380/how-to-tell-if-nic-has-multiqueue-enabled).
  * NIC must have [XDP driver support](https://github.com/xdp-project/xdp-project/blob/master/areas/drivers/README.org) for high-throughput (10 Gbps+).

If you discover that a card not listed in the table above is compatible, please let us know by emailing support [at] libreqos.io.

## Virtual Machine
LibreQoS can be run as a VM, although this comes at a performance penalty of 30%. For VMs, NIC passthrough is required to achieve throughput above 10 Gbps (XDP vs generic XDP).
LibreQoS requires 2 or more RX/TX queues, so when using a virtualization platform such as Proxmox, be sure to enable [Multiqueue](https://forum.proxmox.com/threads/where-is-multiqueue.146783/) for the shaping interfaces assigned to the VM. Multiqueue should be set equal to the number of vCPU cores assigned to the VM.

| Throughput | vCPU* |  RAM  |  Disk |
|:-------:|:-----:|:-----:|:-----:|
| 1 Gbps  | 2     | 8 GB  | 50 GB |
| 10 Gbps | 8     | 32 GB | 50 GB |

* Assumes vCPU performance equal to a single core of the Intel Xeon E-2456 with hyper-threading disabled.
