<p align="center">
  <a href="https://libreqos.io/"><img src="https://github.com/user-attachments/assets/a3675b5a-d109-4e9c-9511-ece7fe6825c1" width="500" alt="LibreQoS"/></a>
</p>

# LibreQoS v2.0

LibreQoS is a self-hosted traffic management and network operations platform for Internet Service Providers and enterprise networks. It improves subscriber experience by reducing latency under load while giving operators a local, topology-aware view of network health, subscriber behavior, queue conditions, and traffic patterns.

LibreQoS runs on a server deployed as a managed bridge between the edge router and the rest of the network. Existing routers, switches, access points, and OLTs do not need to be replaced. A single LibreQoS server can shape traffic for tens of thousands of subscribers while also powering the local Web UI and operational APIs.

## Why operators deploy LibreQoS

- Reduce latency and bufferbloat during congestion using modern flow queueing and active queue management.
- Troubleshoot from the whole network down to an individual subscriber connection in one local operator console.
- See topology, flows, ASN activity, retransmits, RTT, and queue behavior instead of relying on a single black-box score.
- Keep operational control with a self-hosted deployment model, open source core, and documented source-of-truth workflows.
- Add optional long-term analytics, multi-shaper visibility, and support tooling through [LibreQoS Insight](https://libreqos.io/insight/).

## What is new in the v2.0 experience

- **A stronger local operator console.** Dashboard, Site Map, Flow Globe, Network Tree, Queue Dynamics, Queue Stats, and ASN Analysis make day-to-day troubleshooting faster and more explainable.
- **Topology-aware operations.** LibreQoS follows the way ISPs actually run networks: sites, APs, sectors, OLTs, branches, and subscriber hierarchies.
- **Explainable congestion insight.** Queue Dynamics, Queue Tree, RTT, retransmits, utilization, and live flow views help operators see whether a problem is shaping-related, upstream, local RF, or elsewhere.
- **Clear source-of-truth workflows.** Built-in integrations, manual files, and custom pipelines are all documented so operators can keep persistent shaping data under control.
- **Production-focused deployment guidance.** Quickstart, upgrade, troubleshooting, architecture, and Ubuntu 24.04 hotfix documentation are all part of the v2.0 operator workflow.

<a href="https://libreqos.io/"><img alt="LibreQoS dashboard and operator views" src="https://github.com/user-attachments/assets/2f1f0390-2fae-4438-a252-00087569052d"></a>


## How LibreQoS fits into an ISP network

LibreQoS is typically deployed inline as a bridge between the edge router and the network core. It applies shaping locally while collecting the operator-visible data that powers the Web UI and, when enabled, Insight.

LibreQoS helps operators work across several layers of the network:

- **Network-wide visibility:** dashboard views, topology views, site maps, traffic maps, and flow summaries.
- **Topology-aware diagnosis:** tree and sankey views that follow parent/child relationships and show utilization patterns.
- **Subscriber-level troubleshooting:** per-circuit RTT, retransmits, queue stats, queue path, flow evidence, and throughput history.
- **Traffic intelligence:** ASN analysis, geographic flow views, and protocol/application context through the broader analytics stack.

## Open source core with optional commercial layers

LibreQoS itself is open source under GPL2.

Commercial add-ons are available for operators who want more:

- **LibreQoS Insight:** long-term analytics, multi-shaper visibility, richer operational history, support tooling, and additional analysis workflows.
- **LibreQoS API:** paid API package for operators integrating LibreQoS into broader OSS/BSS or automation workflows.

## Documentation and getting started

- [Documentation](https://libreqos.readthedocs.io/en/latest/)
- [Quickstart](https://libreqos.readthedocs.io/en/latest/docs/v2.0/quickstart.html)
- [System Requirements](https://libreqos.readthedocs.io/en/latest/docs/v2.0/requirements.html)
- [Web UI / Node Manager](https://libreqos.readthedocs.io/en/latest/docs/v2.0/node-manager-ui.html)
- [LibreQoS Insight](https://libreqos.readthedocs.io/en/latest/docs/v2.0/insight.html)

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
