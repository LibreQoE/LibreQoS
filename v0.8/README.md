# v0.8 (IPv4 & IPv6) (Stable)

- Released: 2 July 2021

## Features

- Dual stack: client can be shaped by same qdisc for both IPv4 and IPv6

- Up to 1000 clients (IPv4/IPv6)

- Real world asymmetrical throughput: between 2Gbps and 4.5Gbps depending on CPU single thread performance.

- HTB+fq_codel or HTB+cake

- Shape Clients by Access Point / Node capacity

- TC filters split into groups through hashing filters to increase throughput

- Simple client management via csv file

- Simple statistics - table shows top 20 subscribers by packet loss, with APs listed

## Limitations

- Qdisc locking problem limits throughput of HTB used in v0.8 (solved in v0.9). Tested up to 4Gbps/500Mbps asymmetrical throughput using Microsoft Ethr with n=500 streams. High quantities of small packets will reduce max throughput in practice.

- Linux tc hash tables can only handle ~4000 rules each. This limits total possible clients to 1000 in v0.8.
