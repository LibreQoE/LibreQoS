# v0.9 (IPv4) (Stable)

- Released: 11 Jul 2021

## Features

- XDP-CPUMAP-TC integration greatly improves throughput, allows many more IPv4 clients, and lowers CPU use. Latency reduced by half on networks previously limited by single-CPU / TC QDisc locking problem in v.0.8.

- Tested up to 10Gbps asymmetrical throughput on dedicated server (lab only had 10G router). v0.9 is estimated to be capable of an asymmetrical throughput of 20Gbps-40Gbps on a dedicated server with 12+ cores.

- MQ+HTB+fq_codel or MQ+HTB+cake

- Now defaults to 'cake diffserv4' for optimal client performance

- Client limit raised from 1,000 to 32,767

- Shape Clients by Access Point / Node capacity

- APs equally distributed among CPUs / NIC queues to greatly increase throughput

- Simple client management via csv file

## Considerations

- Each Node / Access Point is tied to a queue and CPU core. Access Points are evenly distributed across CPUs. Since each CPU can usually only accommodate up to 4Gbps, ensure any single Node / Access Point will not require more than 4Gbps throughput.

## Limitations

- Not dual stack, clients can only be shaped by IPv4 address for now in v0.9. Once IPv6 support is added to XDP-CPUMAP-TC we can then shape IPv6 as well.

- XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).
