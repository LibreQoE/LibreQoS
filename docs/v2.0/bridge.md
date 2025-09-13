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
    renderer: networkd
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
    renderer: networkd
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
