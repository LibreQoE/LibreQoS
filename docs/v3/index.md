# LibreQoS v3 Documentation

```{warning}
**Beta 12 — pre-release.** LibreQoS v3 is not yet GA. Auto-CAKE is **off by
default**; AF_XDP is **not supported**. Read
[Known limitations](known-limitations.md) before installing. The
[v2.0 documentation archive](../v2.0/index.md) remains available for v2.0
production.
```

**Version: LibreQoS v3 beta 12 (pre-release).** This documentation set covers
LibreQoS **v3** only. For the previous release, see the
[LibreQoS v2.0 documentation (archived, v2.0-only)](../v2.0/index.md).

LibreQoS v3 is a **DPDK-bridge** deployment: the Linux `tc`/eBPF datapath of
v2.0 is **removed**, and a userspace bridge (`bridge_rs`) applies the shaping
world while `lqosd` provides the control plane, the local API, and the dashboard.
Boot is **lqosd-first** (the control plane migrates on-disk state before the
bridge starts). See [What's new in v3](whats-new.md) for the full v2.0→v3
change list, and the [v2.0 archive](../v2.0/index.md) for the retired
kernel-datapath instructions.

Start here:

```{toctree}
:maxdepth: 1

whats-new
known-limitations
install
upgrade-from-v2
operate
troubleshooting
```

## Where the current operator docs live

- **This set** is the operator-facing v3 documentation.
- The package-adjacent detail (exact install targets, file modes, the deb
  payload) lives next to the packaging under `packaging/` in the source tree:
  `packaging/INSTALL.md`, `packaging/UPGRADE.md`, `packaging/BETA_RUNBOOK.md`.
  Where those overlap, this set is the operator entry point and they are the
  packaging reference.

```{warning}
**v2.0-only instructions no longer apply.** If a page describes the kernel
datapath (`tc`, `eBPF`, XDP attach), the `lqos_bakery` scheduler, the Python
refresh engine, or the Flask `lqos_api`, it is **v2.0-only**. Those instructions
are archived under [v2.0](../v2.0/index.md) and must not be followed on a v3
host.
```

## Beta support

v3 beta is opt-in. Collect the support bundle with
`sudo libreqos-support-bundle` and attach it to your support ticket; include the
package version, the bridge build revision, the NIC/PMD and driver, and the
failing step.
