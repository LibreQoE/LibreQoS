# Configure Shaping Bridge

## Choose Bridge Type

There are two options for the bridge to pass data through your two interfaces:

- Option A: Regular Linux Bridge (Recommended)
- Option B: Bifrost XDP-Accelerated Bridge

The regular Linux bridge is recommended for most installations. The Linux Bridge continues to move data even if the lqosd service is in a failed state, making this a generally safer option in scenatios where a backup route is not in place. It works best with Nvidia/Mellanox NICs such as the ConnectX-5 series (which have superior bridge performance), and VM setups using virtualized NICs. The  Bifrost XDP Bridge is recommended for 40G-100G Intel NICs with XDP support.

Below are the instructions to configure Netplan, whether using the Linux Bridge or Bifrost XDP bridge:

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
            dhcp4: no
            dhcp6: no
        ens20:
            dhcp4: no
            dhcp6: no
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

By setting `dhcp4: no` and `dhcp6: no`, the shaping interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

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
            dhcp4: no
            dhcp6: no
        ens20:
            dhcp4: no
            dhcp6: no
    version: 2
```
```{note}
Please be sure to replace ens19 and ens20 in the example above with the correct shaping interfaces. The order of the interfaces does not matter for this section.
```

By setting `dhcp4: no` and `dhcp6: no`, the shaping interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

Then run

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

To use the XDP bridge, please be sure to set `use_xdp_bridge` to `true` in lqos.conf in the [Configuration](configuration.md) section.

## Sandwich Mode (Optional)

Sandwich mode inserts a lightweight veth+bridge pair between your two physical shaping ports. It is useful when you need a compatibility layer or a hard/accurate rate cap.

When to use
- Unsupported NICs or environments where the standard Linux or XDP bridge has quirks (handy for testing and evaluation).
- Bonded NICs/LACP on the physical ports where the extra bridge hop simplifies attach points.
- A hard/accurate limiter in one or both directions (for metered bandwidth or strict caps). The limiter uses HTB with an optional fq_codel child.

How to enable
- Web UI: Configuration → Network Mode → Bridge Mode → Enable “Sandwich Bridge (veth pair)”.
- Then choose limiter direction (None / Download / Upload / Both). Optionally set per‑direction Mbps overrides and enable “Use fq_codel under HTB”.
- Or set it in `/etc/lqos.conf` (see details in [Configuration → Sandwich Mode Settings](configuration.md#sandwich-mode-settings)).

Notes
- This adds a small amount of overhead versus a direct bridge/XDP path, but is generally fine for testing and many production needs.
- `to_internet` remains the ISP‑facing physical interface; `to_network` remains the LAN‑facing physical interface. Sandwich wiring is handled automatically by LibreQoS.
