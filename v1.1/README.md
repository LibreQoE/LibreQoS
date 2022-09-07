# v1.0 (IPv4) (Stable)

Released: 11 Dec 2021

## Features

Can now shape by Site, in addition to by AP and by Client

## Considerations

If you shape by Site, each site is tied to a queue and CPU core. Sites are evenly distributed across CPUs. Since each CPU can usually only accommodate up to 4Gbps, ensure any single Site will not require more than 4Gbps throughput.

If you shape by Acess Point, each Access Point is tied to a queue and CPU core. Access Points are evenly distributed across CPUs. Since each CPU can usually only accommodate up to 4Gbps, ensure any single Access Point will not require more than 4Gbps throughput.

## Limitations

As with 0.9, not yet dual stack, clients can only be shaped by IPv4 address until IPv6 support is added to XDP-CPUMAP-TC. Once that happens we can then shape IPv6 as well.

XDP's cpumap-redirect achieves higher throughput on a server with direct access to the NIC (XDP offloading possible) vs as a VM with bridges (generic XDP).
