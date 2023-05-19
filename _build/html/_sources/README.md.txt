<a href="https://libreqos.io/"><img alt="LibreQoS" src="https://user-images.githubusercontent.com/22501920/202913614-4ff2e506-e645-4a94-9918-d512905ab290.png"></a>

LibreQoS is a Quality of Experience (QoE) Smart Queue Management (SQM) system designed for Internet Service Providers to optimize the flow of their network traffic and thus reduce bufferbloat, keep the network responsive, and improve the end-user experience.

Servers running LibreQoS can shape traffic for many thousands of customers.

Learn more at [LibreQoS.io](https://libreqos.io/)!

## Sponsors

Special thanks to Equinix for providing server resources to support the development of LibreQoS.
Learn more about [Equinix Metal here](https://deploy.equinix.com/metal/).

## Support LibreQoS

Please support the continued development of LibreQoS by sponsoring us via [GitHub Sponsors](https://github.com/sponsors/LibreQoE) or [Patreon](https://patreon.com/libreqos).

## Documentation

[Docs](https://libreqos.readthedocs.io)

## Matrix Chat

Our Matrix chat channel is available at [https://matrix.to/#/#libreqos:matrix.org](https://matrix.to/#/#libreqos:matrix.org).

<img alt="LibreQoS" src="https://user-images.githubusercontent.com/22501920/223866474-603e1112-e2e6-4c67-93e4-44c17b1b7c43.png"></a>

## Features

### Flexible Hierarchical Shaping / Back-Haul Congestion Mitigation

<img src="https://raw.githubusercontent.com/LibreQoE/LibreQoS/main/docs/nestedHTB2.png" width="350"></img>

Starting in version v1.1+, operators can map their network hierarchy in LibreQoS. This enables both simple network hierarchies (Site>AP>Client) as well as much more complex ones (Site>Site>Micro-PoP>AP>Site>AP>Client). This can be used to ensure that a given site’s peak bandwidth will not exceed the capacity of its back-haul links (back-haul congestion control). Operators can support more users on the same network equipment with LibreQoS than with competing QoE solutions which only shape by AP and Client.

### CAKE

CAKE is the product of nearly a decade of development efforts to improve on fq\_codel. With the diffserv\_4 parameter enabled – CAKE groups traffic in to Bulk, Best Effort, Video, and Voice. This means that without having to fine-tune traffic priorities as you would with DPI products – CAKE automatically ensures your clients’ OS update downloads will not disrupt their zoom calls. It allows for multiple video conferences to operate on the same connection which might otherwise “fight” for upload bandwidth causing call disruptions. With work-from-home, remote learning, and tele-medicine becoming increasingly common – minimizing video call disruptions can save jobs, keep students engaged, and help ensure equitable access to medical care.

### XDP

Fast, multi-CPU queueing leveraging xdp-cpumap-tc and cpumap-pping. Currently tested in the real world past 11 Gbps (so far) with just 30% CPU use on a 16 core Intel Xeon Gold 6254. It's likely capable of 30Gbps or more.

### Graphing

You can graph bandwidth and TCP RTT by client and node (Site, AP, etc), using InfluxDB.

### CRM Integrations

- UISP
- Splynx
