# v1.1 (IPv4) (Beta)

Released: 2022

## Installation Guide
- 📄 [LibreQoS v1.1 Installation & Usage Guide Physical Server and Ubuntu 21.10](https://github.com/rchac/LibreQoS/wiki/LibreQoS-v1.1-Installation-&-Usage-Guide-Physical-Server-and-Ubuntu-21.10)

## Features

- Tested up to 11Gbps asymmetrical throughput in real world deployment with 5000+ clients.

- Network hierarchy can be mapped to the network.json file. This allows for both simple network heirarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client).

- Graphing of bandwidth to InfluxDB. Parses bandwidth data from "tc -s qdisc show" command, minimizing CPU use.

- Graphing of TCP latency to InfluxDB - via PPing integration.

## Considerations

- Any top-level parent node is tied to a single CPU core. Top-level nodes are evenly distributed across CPUs. Since each CPU can usually only accommodate up to 4Gbps, ensure any single top-level parent node will not require more than 4Gbps throughput.

## Limitations

- As with 0.9 and v1.0, not yet dual stack, clients can only be shaped by IPv4 address until IPv6 support is added to XDP-CPUMAP-TC. Once that happens we can then shape IPv6 as well.

- XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).
