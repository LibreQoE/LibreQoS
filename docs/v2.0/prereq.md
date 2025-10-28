# Prerequisites

## Servers & Hypervisors

### Disable Hyper-Threading in the BIOS
Disable hyperthreading on the BIOS/UEFI of your host system. Hyperthreaading is also known as Simultaneous Multi Threading (SMT) on AMD systems. Disabling this is very important for optimal performance of the XDP cpumap filtering and, in turn, throughput and latency.

- Boot, pressing the appropriate key to enter the BIOS settings
- For AMD systems, you will have to navigate the settings to find the "SMT Control" setting. Usually it is under something like ```Advanced -> AMD CBS -> CPU Common Options -> Thread Enablement -> SMT Control``` Once you find it, switch to "Disabled" or "Off"
- For Intel systems, you will also have to navigate the settings to find the "hyperthrading" toggle option. On HP servers it's under ```System Configuration > BIOS/Platform Configuration (RBSU) > Processor Options > Intel (R) Hyperthreading Options.```
- Save changes and reboot

### Disable SR-IOV in the BIOS

SR-IOV can disable XDP native (driver mode) on Physical Functions (PFs), forcing XDP Generic (SKB) and reducing performance and stability for LibreQoS. Disable SR-IOV in BIOS/UEFI for NICs used by LibreQoS. If per-slot/per-port options exist, set them to Disabled.

## Hypervisors

### Proxmox

For Proxmox VMs, NIC passthrough is required to achieve throughput above 10 Gbps (XDP vs generic XDP).

LibreQoS requires 2 or more RX/TX queues, so when using Proxmox, please be sure to enable [Multiqueue](https://forum.proxmox.com/threads/where-is-multiqueue.146783/) for the shaping interfaces assigned to the VM. Multiqueue should be set equal to the number of vCPU cores assigned to the VM.

### Hyper-V

#### Hyper-V MAC Spoofing (If Inside a VM)

If your LibreQoS system is running inside Hyper-V and you’ve bridged two vNICs (eth1, eth2) — Hyper-V will block traffic from the bridge because the bridge generates frames with different MACs than the vNIC’s assigned MAC. To fix this on a Windows host:
```
Set-VMNetworkAdapter -VMName "YourLinuxVM" -MacAddressSpoofing On
```
Then restart the VM.
