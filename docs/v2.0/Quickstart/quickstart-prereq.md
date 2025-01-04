# Server Setup - Pre-requisites

Disable hyperthreading on the BIOS/UEFI of your host system. Hyperthreaading is also known as Simultaneous Multi Threading (SMT) on AMD systems. Disabling this is very important for optimal performance of the XDP cpumap filtering and, in turn, throughput and latency.

- Boot, pressing the appropriate key to enter the BIOS settings
- For AMD systems, you will have to navigate the settings to find the "SMT Control" setting. Usually it is under something like ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Once you find it, switch to "Disabled" or "Off"
- For Intel systems, you will also have to navigate the settings to find the "hyperthrading" toggle option. On HP servers it's under ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options.```
- Save changes and reboot

## Install Ubuntu Server

You can download Ubuntu Server 22.04 from <a href="https://ubuntu.com/download/server">https://ubuntu.com/download/server</a>.

1. Boot Ubuntu Server from USB.
2. Follow the steps to install Ubuntu Server.
3. If you use a Mellanox network card, the Ubuntu Server installer will ask you whether to install the mellanox/intel NIC drivers. Check the box to confirm. This extra driver is important.
4. On the Networking settings step, it is recommended to assign a static IP address to the management network interface.
5. Ensure SSH server is enabled so you can more easily log into the server later.
6. You can use scp or sftp to access files from your LibreQoS server for easier file editing. Here's how to access via scp or sftp using an [Ubuntu](https://www.addictivetips.com/ubuntu-linux-tips/sftp-server-ubuntu/) or [Windows](https://winscp.net/eng/index.php) machine.

### Choose Bridge Type

There are two options for the bridge to pass data through your two interfaces:

- Option A: Bifrost XDP-Accelerated Bridge
- Option B: Regular Linux Bridge

The regular Linux bridge is recommended for Nvidia/Mellanox NICs such as the ConnectX-5 series (which have superior bridge performance), and VM setups using virtualized NICs. The Bifrost Bridge is recommended for Intel NICs with XDP support, such as the X520 and X710.

To use the Bifrost bridge, be sure to enable Bifrost/XDP in lqos.conf in the [Configuration](configuration.md) section.

Below are the instructions to configure Netplan, whether using the Linux Bridge or Bifrost XDP bridge:

## Netplan config

### Option A: Netplan config for a regular Linux bridge

Ubuntu Server uses NetPlan, which uses .yaml files in /etc/netplan to determine interface settings.
Here, we will add a .yaml specifically for LibreQoS - that way it is not overwritten when changes are made to the default .yaml file.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Assuming your interfaces are ens19 and ens20, here is what your file would look like:

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

By setting `dhcp4: no` and `dhcp6: no`, the interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

Then run

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```

### Option B: Netplan config for the Bifrost XDP bridge

Ubuntu Server uses NetPlan, which uses .yaml files in /etc/netplan to determine interface settings.
Here, we will add a .yaml specifically for LibreQoS - that way it is not overwritten when changes are made to the default .yaml file.

```shell
sudo nano /etc/netplan/libreqos.yaml
```

Assuming your interfaces are ens19 and ens20, here is what your file would look like:

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

By setting `dhcp4: no` and `dhcp6: no`, the interfaces will be brought up as part of the normal boot cycle, despite not having IP addresses assigned.

Then run

```shell
sudo chmod 600 /etc/netplan/libreqos.yaml
sudo netplan apply
```
