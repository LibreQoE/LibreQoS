# Server Setup - Pre-requisites

Disable hyperthreading on the BIOS/UEFI of your host system. Hyperthreaading is also known as Simultaneous Multi Threading (SMT) on AMD systems. Disabling this is very important for optimal performance of the XDP cpumap filtering and, in turn, throughput and latency.

- Boot, pressing the appropriate key to enter the BIOS settings
- For AMD systems, you will have to navigate the settings to find the "SMT Control" setting. Usually it is under something like ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Once you find it, switch to "Disabled" or "Off"
- For Intel systems, you will also have to navigate the settings to find the "hyperthrading" toggle option. On HP servers it's under ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options.```
- Save changes and reboot

## Install Ubuntu Server

We recommend Ubuntu Server because its kernel version tends to track closely with the mainline Linux releases. Our current documentation assumes Ubuntu Server. To run LibreQoS v1.4, Linux kernel 5.11 or greater is required, as 5.11 includes some important XDP patches. Ubuntu Server 22.04 uses kernel 5.13, which meets that requirement.

You can download Ubuntu Server 22.04 from <a href="https://ubuntu.com/download/server">https://ubuntu.com/download/server</a>.

1. Boot Ubuntu Server from USB.
2. Follow the steps to install Ubuntu Server.
3. If you use a Mellanox network card, the Ubuntu Server installer will ask you whether to install the mellanox/intel NIC drivers. Check the box to confirm. This extra driver is important.
4. On the Networking settings step, it is recommended to assign a static IP address to the management NIC.
5. Ensure SSH server is enabled so you can more easily log into the server later.
6. You can use scp or sftp to access files from your LibreQoS server for easier file editing. Here's how to access via scp or sftp using an [Ubuntu](https://www.addictivetips.com/ubuntu-linux-tips/sftp-server-ubuntu/) or [Windows](https://winscp.net/eng/index.php) machine.

### Choose Bridge Type

There are two options for the bridge to pass data through your two interfaces:

- Bifrost XDP-Accelerated Bridge
- Regular Linux Bridge


The regular Linux bridge is recommended for Nvidea/Mellanox NICs such as the ConnectX-5 series (which have superior bridge performance), and VM setups using virtualized NICs. The Bifrost Bridge is recommended for Intel NICs with XDP support, such as the X520 and X710.

To use the Bifrost bridge, be sure to enable Bifrost/XDP in lqos.conf in the [Configuration](configuration.md) section.

Below are the instructions to configure Netplan, whether using the Linux Bridge or Bifrost XDP bridge:

## Netplan config

### Netplan for a regular Linux bridge (if not using Bifrost XDP bridge)

From the Ubuntu VM, create a linux interface bridge - br0 - with the two shaping interfaces.
Find your existing .yaml file in /etc/netplan/ with

```shell
cd /etc/netplan/
ls
```

Then edit the .yaml file there with

```shell
sudo nano XX-cloud-init.yaml
```

With XX corresponding to the name of the existing file.

Editing the .yaml file, we need to define the shaping interfaces (here, ens19 and ens20) and add the bridge with those two interfaces. Assuming your interfaces are ens18, ens19, and ens20, here is what your file might look like:

```yaml
# This is the network config written by 'subiquity'
network:
  ethernets:
    ens18:
      addresses:
      - (addr goes here)
      gateway4: (gateway goes here)
      nameservers:
        addresses:
        - 1.1.1.1
        - 8.8.8.8
        search: []
    ens19:
      dhcp4: no
      dhcp6: no
    ens20:
      dhcp4: no
      dhcp6: no
  version: 2
  bridges:
    br0:
      interfaces:
        - ens19
        - ens20
```

Make sure to replace `(addr goes here)` with your LibreQoS VM's address and subnet CIDR, and to replace `(gateway goes here)` with whatever your default gateway is.

Then run

```shell
sudo netplan apply
```

### Netplan for the Bifrost XDP bridge

Find your existing .yaml file in /etc/netplan/ with

```shell
cd /etc/netplan/
ls
```

Then edit the .yaml file there with

```shell
sudo nano XX-cloud-init.yaml
```

With XX corresponding to the name of the existing file.

Editing the .yaml file, we need to define the shaping interfaces (here, ens19 and ens20) and add the bridge with those two interfaces. Assuming your interfaces are ens18, ens19, and ens20, here is what your file might look like:

```
network:
  ethernets:
    ens18:
      addresses:
      - (addr goes here)
      gateway4: (gateway goes here)
      nameservers:
        addresses:
        - (etc)
        search: []
    ens19:
      dhcp4: off
      dhcp6: off
    ens20:
      dhcp4: off
      dhcp6: off
```

By setting `dhcp4: off` and `dhcp6: off`, bringing them up but not assigning addresses is part of the normal boot cycle.

Make sure to replace (addr goes here) with your LibreQoS VM's address and subnet CIDR, and to replace `(gateway goes here)` with whatever your default gateway is.
Once everything is in place, run:

```shell
sudo netplan apply
```
