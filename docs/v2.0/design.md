# Network Design Assumptions

## Officially supported configuration

- LibreQoS is placed inline at the edge of your network, usually between the network's border router (NAT, firewall) and the core distribution router / switch.
- If you use NAT/CG-NAT, place LibreQoS inline prior to where NAT is applied, as LibreQoS needs to shape pre-NAT addresses (100.64.0.0/12) not public post-NAT IPs.
- For networks using MPLS: LibreQoS can parse MPLS traffic, but the traffic must follow the standard pattern (mpls tags)(optional vlan tags)(ip header). If you use MPLS with a different pattern, you would ideally want to terminate MPLS traffic at the core distribution router / switch, before it reaches LibreQoS.
- OSPF primary link (low cost) through the server running LibreQoS
- OSPF backup link (high cost, maybe 200 for example)

![Offical Configuration](https://github.com/user-attachments/assets/ae0ff660-8de0-413e-a83a-d62c173447a4)

## Testbed configuration
When you are first testing out LibreQoS, we recommend deploying a small-scale testbed to see it in action.
![image](https://github.com/user-attachments/assets/6174bd29-112d-4b00-bea8-41314983d37a)

### Network Interface Card

```{note}
You must have one of these:
- single NIC with two interfaces,
- two NICs with single interface,
- 2x VLANs interface (using one or two NICs).
```

LibreQoS requires NICs to have 2 or more RX/TX queues and XDP support. While many cards theoretically meet these requirements, less commonly used cards tend to have unreported driver bugs which impede XDP functionality and make them unusable for our purposes. At this time we recommend the Intel x520, Intel x710, and Nvidia (ConnectX-5 or newer) NICs. We cannot guarantee compatibility with other cards.

## Alternate configuration

This alternate configuration uses Spanning Tree Protocol (STP) to modify the data path in the event the LibreQoS device is offline for maintenance or another problem.

```{note}
Most of the same considerations apply to the alternate configuration as they do to the officially supported configuation
```

- LibreQoS is placed inline at the edge of your network, usually between the network's border router (NAT, firewall) and the core distribution router / switch.
- If you use NAT/CG-NAT, place LibreQoS inline prior to where NAT is applied, as LibreQoS needs to shape pre-NAT addresses (100.64.0.0/12) not public post-NAT IPs.
- For networks using MPLS: LibreQoS can parse MPLS traffic, but the traffic must follow the standard pattern (mpls tags)(optional vlan tags)(ip header). If you use MPLS with a different pattern, you would ideally want to terminate MPLS traffic at the core distribution router / switch, before it reaches LibreQoS.
- Spanning Tree primary link (low cost) through the server running LibreQoS
- Spanning Tree backup link (high cost, maybe 80 for example)

Keep in mind that if you use different bandwidth links, for example, 10 Gbps through LibreQoS, and 1 Gbps between core switch and edge router, you may need to be more intentional with your STP costs.

![image](https://github.com/user-attachments/assets/39247655-3bcf-4a0c-8cfb-1ed1a96d3c2d)
