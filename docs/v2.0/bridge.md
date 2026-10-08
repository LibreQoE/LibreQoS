# Configure Shaping Bridge

## Choose Bridge Type

There are two options for the bridge to pass data through your two interfaces:

- Option A: Regular Linux Bridge (Recommended)
- Option B: Bifrost XDP-Accelerated Bridge

The regular Linux bridge is recommended for most installations. The Linux Bridge continues to move data even if the lqosd service is in a failed state, making this a generally safer option in scenarios where a backup route is not in place. It works best with Nvidia/Mellanox NICs such as the ConnectX-5 series (which have superior bridge performance), and VM setups using virtualized NICs. The Bifrost XDP Bridge is recommended for 40G-100G Intel NICs with XDP support.

Below are the instructions to configure Netplan, whether using the Linux Bridge or Bifrost XDP bridge:

```{note}
The Network Mode page in the LibreQoS web UI inspects the current Netplan files and offers eligible non-management interfaces. Linux bridge and single-interface modes can stage and apply managed `libreqos.yaml` changes with a timed rollback window. XDP mode saves only `lqos.conf`; it never generates or applies Netplan.
```

```{note}
First-run setup offers Linux Bridge, XDP Bridge, and Single Interface. When using XDP with a bond, configure the bond in Netplan first and select the bond master in LibreQoS. Do not select an individual bond member.
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

### XDP bridge with 802.3ad bonds

The Linux bonding driver supports native XDP on `802.3ad` bond masters when every member driver also supports native XDP. Configure LACP on the connected switch ports and define the bonds in Netplan before selecting them in LibreQoS. The following example uses one two-port bond on each side of the shaper:

```yaml
network:
    ethernets:
        enp1s0:
            dhcp4: false
            dhcp6: false
        enp2s0:
            dhcp4: false
            dhcp6: false
        enp3s0:
            dhcp4: false
            dhcp6: false
        enp4s0:
            dhcp4: false
            dhcp6: false
    bonds:
        bond-wan:
            interfaces: [enp1s0, enp2s0]
            parameters:
                mode: 802.3ad
        bond-lan:
            interfaces: [enp3s0, enp4s0]
            parameters:
                mode: 802.3ad
    version: 2
```

Then select `bond-wan` as the Internet-facing interface and `bond-lan` as the LAN-facing interface. The equivalent `lqos.conf` section is:

```toml
[bridge]
use_xdp_bridge = true
to_internet = "bond-wan"
to_network = "bond-lan"
```

Do not select `enp1s0` through `enp4s0` in LibreQoS. They are bond members; XDP must attach to the bond masters. Confirm that each bond exposes multiple RX/TX queues and that `lqosd` attaches in native driver mode before carrying production traffic. The Linux kernel documentation lists the [bonding modes that support native XDP](https://docs.kernel.org/networking/bonding.html#what-bonding-modes-support-native-xdp).

After changing an existing node to use the bond masters, restart `lqosd` so it attaches XDP to the newly selected interfaces:

```shell
sudo systemctl restart lqosd
```
