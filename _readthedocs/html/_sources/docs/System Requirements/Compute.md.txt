## System Requirements
### VM or physical server
* For VMs, NIC passthrough is required for optimal throughput and latency (XDP vs generic XDP). Using Virtio / bridging is much slower than NIC passthrough. Virtio / bridging should not be used for large amounts of traffic.

### CPU
* 2 or more CPU cores
* A CPU with solid [single-thread performance](https://www.cpubenchmark.net/singleThread.html#server-thread) within your budget. Queuing is very CPU-intensive, and requires high single-thread performance.

Single-thread CPU performance will determine the max throughput of a single HTB (cpu core), and in turn, what max speed plan you can offer customers.

| Customer Max Plan   | Passmark Single-Thread   |
| --------------------| ------------------------ |
| 100 Mbps            | 1000                     |
| 250 Mbps            | 1500                     |
| 500 Mbps            | 2000                     |
| 1 Gbps              | 2500                     |
| 2 Gbps              | 3000                     |

Below is a table of approximate aggregate throughput capacity, assuming a a CPU with a [single thread](https://www.cpubenchmark.net/singleThread.html#server-thread) performance of 2700 or greater:

| Aggregate Throughput    | CPU Cores     |
| ------------------------| ------------- |
| 500 Mbps                | 2             |
| 1 Gbps                  | 4             |
| 5 Gbps                  | 6             |
| 10 Gbps                 | 8             |
| 20 Gbps                 | 16            |
| 50 Gbps*                | 32            |

(* Estimated)

So for example, an ISP delivering 1Gbps service plans with 10Gbps aggregate throughput would choose a CPU with a 2500+ single-thread score and 8 cores, such as the Intel Xeon E-2388G @ 3.20GHz.

### Memory
* Minimum RAM = 2 + (0.002 x Subscriber Count) GB
* Recommended RAM:

| Subscribers   | RAM           |
| ------------- | ------------- |
| 100           | 4 GB          |
| 1,000         | 8 GB          |
| 5,000         | 16 GB         |
| 10,000*       | 18 GB         |
| 50,000*       | 24 GB         |

(* Estimated)

### Server Recommendations
It is most cost-effective to buy a used server with specifications matching your unique requirements, as laid out in the System Requirements section below.
For those who do not have the time to do that, here are some off-the-shelf options to consider:
* 1 Gbps | [Supermicro SuperServer E100-9W-L](https://www.thinkmate.com/system/superserver-e100-9w-l)
* 10 Gbps | [Supermicro SuperServer 510T-ML (Choose E-2388G)](https://www.thinkmate.com/system/superserver-510t-ml)
* 20 Gbps | [Dell R450 Config](https://www.dell.com/en-us/shop/servers-storage-and-networking/poweredge-r450-rack-server/spd/poweredge-r450/pe_r450_15127_vi_vp?configurationid=a7663c54-6e4a-4c96-9a21-bc5a69d637ba)

The [AsRock 1U4LW-B6502L2T](https://www.thinkmate.com/system/asrock-1u4lw-b6502l2t/635744) can be a great lower-cost option as well.
