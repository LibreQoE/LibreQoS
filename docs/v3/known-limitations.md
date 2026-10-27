# Known limitations — LibreQoS v3 beta 12

**Beta 12 is a pre-release.** LibreQoS v3 is not yet GA. For production v2.0,
use the [v2.0 documentation archive](../v2.0/index.md) and the v2.0 package
series. This page lists the v3 beta limitations that matter before an install
or upgrade.

## Auto-CAKE is OFF by default

Dynamic (auto) CAKE is **off by default** in beta 12. The live CAKE kind-flip
path is still under development and is **not supported** in this beta. Keep `LIBREQOS_DPDK_DYNAMIC_CAKE=0` (the default); opting in requires
`LIBREQOS_DPDK_DYNAMIC_CAKE=1`, and dynamic CAKE remains unsupported in this
beta. Do not enable it on a production host. A static `cake_core` leaf is
unaffected.

## AF_XDP is not supported

The AF_XDP dataplane (`--vdev=net_af_xdp…`) is **not supported** in beta 12.
The preflight blocks an AF_XDP-configured dataplane by default; the explicit
lab-only override `LQOS_PREFLIGHT_ALLOW_UNSUPPORTED_PMD=1` turns that block into
a warning. Use a supported PMD instead (below).

## Supported dataplane PMDs and hardware

Supported dataplane classes in beta 12:

- `ixgbe` (Intel X520/82599-class 10G)
- `ice` + `iavf` (Intel E810 and E810 VFs)
- `i40e` (Intel X710/X722-class)
- `mlx5` (Mellanox ConnectX-class)
- **Virtio**, within the qualified matrix: modern virtio 1.0
  (`0x1af4:0x1041`), at least 2 RX and 2 TX queues, with `mrg_rxbuf` and
  `guest_csum`. Legacy or under-configured virtio is not supported.

`net_e1000` is **packaged** in beta 12 for the Intel e1000/igb/igc family, but
it is **not hardware-qualified** yet: no I210/I350/I225/I226 support claim is
made. E810 support is unchanged.

## On-a-stick

Single-port **on-a-stick** is supported with a preflight warning: WAN and LAN
share one port through two VLANs. There is no port redundancy and throughput is
shared. Configure the LAN/WAN VLAN IDs and the peer trunk. A configured inline
pair that resolves to only one supported port is blocked by the preflight.

## Performance claims

Beta 12 performance claims are limited to the documented release test matrix.
Long-run performance characterization is still in progress; do not extrapolate
a claim from a short run or from the beta documentation.

The v2.0 pages are preserved in this documentation build under `../v2.0/`;
the RTD `stable` version is the v2.2 release.

## Reporting beta issues

Collect the support bundle and attach it to your support ticket:

```bash
sudo libreqos-support-bundle
```

Include the package version, the bridge build revision, the NIC/PMD and driver,
and the step that failed. For v2.0 production, see the
[v2.0 documentation archive](../v2.0/index.md).
