# Network Design Assumptions

## Officially supported configuration

- LibreQoS placed inline in network, usually between an edge router (NAT, firewall) and core router (distribution to sites across network).
  - If you use NAT/CG-NAT, place LibreQoS inline south of where NAT is applied, as LibreQoS needs to shape internal addresses (100.64.0.0/12) not public post-NAT IPs.
- Edge and Core routers should have 1500 MTU on links between them
- If you use MPLS, you would terminate MPLS traffic at the core router. LibreQoS cannot decapsulate MPLS on its own.
- OSPF primary link (low cost) through the server running LibreQoS
- OSPF backup link (high cost, maybe 200 for example)

![Offical Configuration](https://raw.githubusercontent.com/rchac/LibreQoS/main/docs/design.png)

### Network Interface Card

```{note}
You must have one of these:
- single NIC with two interfaces,
- two NICs with single interface,
- 2x VLANs interface (using one or two NICs).
```

LibreQoS requires NICs to have 2 or more RX/TX queues and XDP support. While many cards theoretically meet these requirements, less commonly used cards tend to have unreported driver bugs which impede XDP functionality and make them unusable for our purposes. At this time we recommend the Intel x520, Intel x710, and Nvidia (ConnectX-5 or newer) NICs. We cannot guarantee compatibility with other cards.

## Alternate configuration (Not officially supported)

This alternate configuration uses Spanning Tree Protocol (STP) to modify the data path in the event the LibreQoS device is offline for maintenance or another problem.

```{note}
Most of the same considerations apply to the alternate configuration as they do to the officially supported configuation
```

- LibreQoS placed inline in network, usually between an edge router (NAT, firewall) and core router (distribution to sites across network).
  - If you use NAT/CG-NAT, place LibreQoS inline south of where NAT is applied, as LibreQoS needs to shape internal addresses (100.64.0.0/12) not public post-NAT IPs.
- Edge router and Core switch should have 1500 MTU on links between them
- If you use MPLS, you would terminate MPLS traffic somewhere south of the core/distribution switch. LibreQoS cannot decapsulate MPLS on its own.
- Spanning Tree primary link (low cost) through the server running LibreQoS
- Spanning Tree backup link (high cost, maybe 80 for example)

Keep in mind that if you use different bandwidth links, for example, 10 Gbps through LibreQoS, and 1 Gbps between core switch and edge router, you may need to be more intentional with your STP costs.

![Alternate Configuration](../stp-diagram.png)
