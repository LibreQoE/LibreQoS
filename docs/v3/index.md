# LibreQoS v3 Documentation

**Version: LibreQoS v3.** These pages cover v3 only. For the previous
release, see the [LibreQoS v2.0 documentation](../v2.0/index.md).

In v3 the Linux `tc`/eBPF datapath of v2.0 is gone. A userspace DPDK bridge
does the shaping, and `lqosd` provides the control plane, the local API and the
dashboard. See [What's new in v3](whats-new.md) for the full list of changes.

```{toctree}
:maxdepth: 1

whats-new
requirements
ubuntu-server
install
upgrade-from-v2
operate
power
integrations
radius
https-caddy
node-manager-i18n
known-limitations
troubleshooting
```

```{warning}
**v2.0 instructions do not apply to v3.** Pages that describe the kernel
datapath (`tc`, eBPF, XDP), the `lqos_bakery` scheduler, the Python refresh
engine, or the Flask `lqos_api` are v2.0-only. Do not follow them on a v3 host.
```
