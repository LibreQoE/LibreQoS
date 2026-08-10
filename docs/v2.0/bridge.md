# Configure Shaping Bridge

## Choose Bridge Type

LibreQoS supports these paths between the two shaping interfaces:

- Option A: Regular Linux Bridge (Recommended)
- Option B: Bifrost XDP-Accelerated Bridge
- Option C: Interface Compatibility Shim for bonds and drivers that cannot host XDP

The regular Linux bridge is recommended for most installations. The Linux Bridge continues to move data even if the lqosd service is in a failed state, making this a generally safer option in scenarios where a backup route is not in place. It works best with Nvidia/Mellanox NICs such as the ConnectX-5 series (which have superior bridge performance), and VM setups using virtualized NICs. The Bifrost XDP Bridge is recommended for 40G-100G Intel NICs with XDP support.

The Netplan examples below cover the regular Linux bridge and direct Bifrost XDP paths. The compatibility shim is described separately afterward.

```{note}
The Network Mode page in the LibreQoS web UI now inspects the current Netplan files, offers eligible non-management interfaces in dropdowns, stages managed `libreqos.yaml` changes for Linux bridge and single-interface modes, applies them with a timed LibreQoS rollback window, and lets you confirm or revert the pending change. You can also restore the previous managed backup from that page. XDP bridge mode remains a manual Netplan workflow.
```

```{note}
First-run setup offers Linux Bridge, Interface Compatibility Shim, and Single Interface. Use the shim only for bonds or drivers that cannot host LibreQoS XDP directly. If LibreQoS detects an existing direct-XDP deployment, setup also preserves that legacy mode and warns before you migrate away from it.
```

```{note}
If a timed Netplan change briefly interrupts your browser session, return to the Network Mode page after connectivity comes back. LibreQoS will resume the pending confirm or revert flow from that page automatically.
```

## Option A: Netplan config for a regular Linux bridge (Recommended)

Ubuntu Server uses NetPlan, which uses .yaml files in /etc/netplan to determine interface settings.
Here, we will add a .yaml specifically for LibreQoS - that way it is not overwritten when changes are made to the default .yaml file.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Assuming your shaping interfaces are ens19 and ens20, here is what your file would look like:

```yaml
network:
    ethernets:
        ens19:
            dhcp4: false
            dhcp6: false
        ens20:
            dhcp4: false
            dhcp6: false
    bridges:
        br0:
            interfaces:
            - ens19
            - ens20
    version: 2
```
```{note}
Please be sure to replace ens19 and ens20 in the example above with the correct shaping interfaces. The order of the interfaces does not matter for this section.
```

By setting `dhcp4: false` and `dhcp6: false`, the shaping interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

Then run

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

## Option B: Netplan config for the Bifrost XDP bridge

Ubuntu Server uses NetPlan, which uses .yaml files in /etc/netplan to determine interface settings.
Here, we will add a .yaml specifically for LibreQoS - that way it is not overwritten when changes are made to the default .yaml file.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Assuming your shaping interfaces are ens19 and ens20, here is what your file would look like:

```yaml
network:
    ethernets:
        ens19:
            dhcp4: false
            dhcp6: false
        ens20:
            dhcp4: false
            dhcp6: false
    version: 2
```
```{note}
Please be sure to replace ens19 and ens20 in the example above with the correct shaping interfaces. The order of the interfaces does not matter for this section.
```

By setting `dhcp4: false` and `dhcp6: false`, the shaping interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

Then run

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

To use the XDP bridge, please be sure to set `use_xdp_bridge` to `true` in lqos.conf in the [Configuration](configuration.md) section.

## Interface Compatibility Shim

Use the interface compatibility shim only when LibreQoS cannot attach XDP directly to the selected interfaces. Common examples are bonded interfaces and NIC drivers without the XDP support LibreQoS requires. Direct XDP or the regular Linux bridge remains preferable when either works.

The shim connects each physical interface to a multiqueue veth through a small Linux bridge. LibreQoS attaches its existing XDP and queueing path to the veth interfaces. This adds CPU overhead, but leaves checksum, segmentation, and VLAN offloads enabled on the physical interfaces. LibreQoS still applies the configured interrupt-coalescing values where the physical driver accepts them.

Choose `Interface Compatibility Shim` during first-run setup, enable it on the `Bridge & Interface Mode` page, or set:

```toml
[bridge]
use_xdp_bridge = true
compatibility_shim = true
to_internet = "bond0"
to_network = "bond1"
```

Restart `lqosd` after changing this setting. LibreQoS chooses the veth queue count from the active shaping CPU count and any configured queue override. It uses the smaller physical-interface MTU for the shim path.

The compatibility shim does not add a link-speed cap, HTB limiter, or `fq_codel` to the physical interfaces. Configure subscriber shaping through the normal LibreQoS queue settings.
