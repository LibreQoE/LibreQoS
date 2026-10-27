<p align="center">
  <a href="https://libreqos.io/"><img src="https://github.com/user-attachments/assets/a3675b5a-d109-4e9c-9511-ece7fe6825c1" width="500" alt="LibreQoS"/></a>
</p>

# LibreQoS

LibreQoS is a self-hosted traffic management and network operations platform for Internet Service Providers and enterprise networks. It improves subscriber experience by reducing latency under load while giving operators a local, topology-aware view of network health, subscriber behavior, queue conditions, and traffic patterns.

LibreQoS runs on a server deployed as a managed bridge between the edge router and the rest of the network. Existing routers, switches, access points, and OLTs do not need to be replaced. A single LibreQoS server can shape traffic for tens of thousands of subscribers while also powering the local Web UI and operational APIs.

## Why operators deploy LibreQoS

- Reduce latency and bufferbloat during congestion using modern flow queueing and active queue management.
- Troubleshoot from the whole network down to an individual subscriber connection in one local operator console.
- See topology, flows, ASN activity, retransmits, RTT, and queue behavior instead of relying on a single black-box score.
- Keep operational control with a self-hosted deployment model, open source core, and documented source-of-truth workflows.
- Add optional long-term analytics, multi-shaper visibility, and support tooling through [LibreQoS Insight](https://libreqos.io/insight/).

## What is new in LibreQoS v3

LibreQoS v3 is a ground-up dataplane change; here is what it means in the operator's terms:

1. **Match subscribers by more than IP.** Classify traffic by CIDR / IPv4 / IPv6 **and** by VLAN tag, VXLAN (VNI + VLAN), or MPLS label — so you can correlate and shape subscribers whose identity isn't visible at L3. A tunnel/VLAN/MPLS miss is counted and never falls through to the IP lookup, so reused inner /24s cannot cross-customer misroute.
2. **Lower latency under load, out of the box.** CAKE is the default queue discipline, and dynamic CAKE allocation is on by default. Idle circuits hold a cheap FIFO leaf and switch to CAKE only when they carry traffic.
3. **Jumbo frame support.** The dataplane now supports MTU up to 9000-byte frames.
4. **Multi-core CAKE.** Each worker core runs a full CAKE instance over its flow subset, with the rate shared across instances instead of serializing on one core.
5. **Site sharding across CPU cores.** Site subtrees are distributed across cpu cores, so a large topology uses the whole server.
6. **Subscriber plans up to 25 Gbps supported.** Shard-tin spreads a single circuit across worker cores — each running a full CAKE instance against a shared rate domain — so a high-rate plan is not capped by one core's CAKE throughput. Per-flow ordering is preserved.
7. **CPU auto-balancing.** The bridge moves circuits between workers as load shifts, instead of leaving one core hot and the rest idle.
8. **100G-class shaping on the right hardware.** Measured at ≥133 Gbps on commodity hardware (AMD Ryzen 9 7950X).
9. **One command for support.** `libreqos-support-bundle` gathers systemd/journal state, the bridge log and leafstats, LKG metadata, config, hugepages, links, `lspci`, and `dpdk-devbind` — read-only, and it never restarts anything.

Read the full [What's new in v3](docs/v3/whats-new.md) change list for the technical detail.

<a href="https://libreqos.io/"><img alt="LibreQoS dashboard and operator views" src="https://github.com/user-attachments/assets/2f1f0390-2fae-4438-a252-00087569052d"></a>

## Installing LibreQoS v3

The install procedure — host requirements, the preflight checker, the apt archive, the configuration files, and the first boot — is maintained in the documentation: **[Installing LibreQoS v3](https://libreqos.readthedocs.io/en/latest/docs/v3/install.html)**. Upgrading an existing v2.0 host? Follow **[Upgrading from v2.0](https://libreqos.readthedocs.io/en/latest/docs/v3/upgrade-from-v2.html)**.

## How LibreQoS fits into an ISP network

LibreQoS is typically deployed inline as a bridge between the edge router and the network core. It applies shaping locally while collecting the operator-visible data that powers the Web UI and, when enabled, Insight.

LibreQoS helps operators work across several layers of the network:

- **Network-wide visibility:** dashboard views, topology views, site maps, traffic maps, and flow summaries.
- **Topology-aware diagnosis:** tree and sankey views that follow parent/child relationships and show utilization patterns.
- **Subscriber-level troubleshooting:** per-circuit RTT, retransmits, queue stats, queue path, flow evidence, and throughput history.
- **Traffic intelligence:** ASN analysis, geographic flow views, and protocol/application context through the broader analytics stack.

## Open source core with optional commercial layers

LibreQoS is open source under GPL-2.0. See [LICENSE](LICENSE).

Commercial add-ons are available for operators who want more:

- **LibreQoS Insight:** long-term analytics, multi-shaper visibility, richer operational history, support tooling, and additional analysis workflows.
- **LibreQoS API:** paid API package for operators integrating LibreQoS into broader OSS/BSS or automation workflows.

## Documentation and getting started

- [LibreQoS v3 documentation](https://libreqos.readthedocs.io/en/latest/)
- [What's new in v3](https://libreqos.readthedocs.io/en/latest/docs/v3/whats-new.html)
- [Installing LibreQoS v3](https://libreqos.readthedocs.io/en/latest/docs/v3/install.html)
- [Upgrading from v2.0](https://libreqos.readthedocs.io/en/latest/docs/v3/upgrade-from-v2.html)
- [Operating LibreQoS v3](https://libreqos.readthedocs.io/en/latest/docs/v3/operate.html)
- [Troubleshooting](https://libreqos.readthedocs.io/en/latest/docs/v3/troubleshooting.html)
- [LibreQoS v2.0 documentation (archived)](https://libreqos.readthedocs.io/en/latest/docs/v2.0/index.html)

## LibreQoS Insight

LibreQoS Insight is the successor to the older Long Term Stats service. It is built for operators who want higher-resolution history, multi-shaper visibility, and deeper operational analytics across RTT, retransmits, throughput, flows, ASN activity, geographic endpoint activity, protocol mix, ethertype, CAKE statistics, and utilization.

Insight provides a free 30-day trial so operators can evaluate its workflows on their own networks. Learn more at [LibreQoS Insight](https://libreqos.io/insight/).

<a href="https://libreqos.io/insight"><img alt="LibreQoS Insight" src="https://github.com/user-attachments/assets/72a61d36-4ee5-438e-98c2-ba3203ea2df9"></a>

## Sponsors

LibreQoS development is made possible by our sponsors, the [NLnet Foundation](https://nlnet.nl/) and Equinix.

LibreQoS has been funded through the NGI0 Entrust Fund, a fund established by NLnet with financial support from the European Commission's Next Generation Internet programme, under the aegis of DG Communications Networks, Content and Technology under grant agreement No 101069594. Learn more at https://nlnet.nl/project/LibreQoS/

## LibreQoS Chat

Our chat server is available at [https://chat.libreqos.io/](https://chat.libreqos.io/).

## LibreQoS Social

- https://www.youtube.com/@LibreQoS
- https://www.linkedin.com/company/libreqos/
- https://www.facebook.com/libreqos
- https://twitter.com/libreqos
- https://fosstodon.org/@LibreQoS/

## In loving memory of Dave Taht

Dave served as LibreQoS' Chief Science Officer and championed the global fight against bufferbloat. You can learn more about [Dave's legacy here](https://libreqos.io/2025/04/01/in-loving-memory-of-dave/), and how the team is working to [carry on his mission](https://libreqos.io/company/).

## External Pull Request Policy

We can only accept PRs that address one specific change or topic each. Please keep changes small and focused per PR to help review and testing.
