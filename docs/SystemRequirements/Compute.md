## System Requirements
### VM or physical server
* Using a dedicated, physical server for LibreQoS is highly recommended.
* For VMs, NIC passthrough is required for optimal throughput and latency (XDP vs generic XDP). Using Virtio / bridging is much slower than NIC passthrough. Virtio / bridging should not be used for large amounts of traffic.

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
* Minimum RAM = 2 + (0.002 x Subscriber Count) GB
* Recommended RAM:

| Subscribers   | RAM           |
| ------------- | ------------- |
| 100           | 4 GB          |
| 1,000         | 8 GB          |
| 5,000         | 16 GB         |
| 10,000        | 32 GB         |
| 50,000*       | 64 GB         |

(* Estimated)

### Server Recommendations
It is most cost-effective to buy a used server with specifications matching your unique requirements, as laid out in the System Requirements section above.
For those who do not have the time to do that, here are some off-the-shelf options to consider:

|   Aggregate   | 100Mbps Plans |  1Gbps Plans  |  2Gbps Plans  |
| ------------- | ------------- | ------------- | ------------- |
| 1 Gbps Total  |       A       |               |               |
| 10 Gbps Total |    B or C     |    B or C     |       C       |

* A | [Lanner L-1513-4C](https://www.whiteboxsolution.com/product/l-1513/) (Select L-1513-4C)
* B | [Supermicro SuperServer 510T-ML](https://www.thinkmate.com/system/superserver-510t-ml) (Select E-2388G)
* C | [Supermicro AS-1015A-MT](https://store.supermicro.com/us_en/as-1015a-mt.html) (Ryzen 9 7700X, 2x16GB DDR5 4800MHz ECC, 1xSupermicro 10-Gigabit XL710+ X557)
