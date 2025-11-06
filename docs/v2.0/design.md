# Deployment Scenarios

## Standard Inline Deployment

LibreQoS is placed inline at the edge of your network, usually between the network's border router (NAT, firewall) and the core distribution router / switch.

#### NAT/CG-NAT
If you use NAT/CG-NAT, please place LibreQoS inline prior to where NAT is applied, as LibreQoS needs to shape pre-NAT addresses (100.64.0.0/12) not public post-NAT IPs.

#### MPLS/VPLS
LibreQoS can parse MPLS traffic, however the traffic must follow the standard pattern:
```
(mpls tags)(optional vlan tags)(ip header)
```
If you use MPLS with a different tag pattern, you would ideally want to terminate MPLS traffic at the core distribution router / switch, before it reaches LibreQoS.

#### Dynamic Routing: Bypass path
- Primary path (low cost) *through* the server running LibreQoS
- Backup path (high cost) *bypassing* the server running LibreQoS.

#### Diagram
![Offical Configuration](https://github.com/user-attachments/assets/e5914a58-3ec6-4eb1-b016-8a57582dd082)

### Option 1: Using Dynamic Routing (Strongly Recommended)

We recommend using dynamic routing protocols such as OSPF to create a high cost and low cost path between the edge router and core distribution router/switch. The low cost path should pass "through" the LibreQoS shaper bridge interfaces to allow LibreQoS to observe and shape traffic. For example, a low cost OSPF path may be set to a value of 1. The high-cost (backup) link would completely bypass LibreQoS, being set to a higher cost (perhaps 100 for OSPF) to ensure that traffic only takes that path when the LibreQoS shaper bridge is not operational.

### Option 2: Using Spanning Tree Protocol (Not Recommended)

You can also use the Spanning Tree Protocol with path costs if OSPF or another dynamic routing protocol is not an option.

```{note}
Most of the same considerations apply to the alternate configuration as they do to the officially supported configuation
```

Keep in mind that if you use different bandwidth links, for example, 10 Gbps through LibreQoS, and 1 Gbps between core switch and edge router, you may need to be more intentional with your STP costs.

## Testbed Deployment (Optional)
When you are first testing out LibreQoS, you can deploy a small-scale testbed to see it in action on a test network.
![image](https://github.com/user-attachments/assets/6174bd29-112d-4b00-bea8-41314983d37a)
