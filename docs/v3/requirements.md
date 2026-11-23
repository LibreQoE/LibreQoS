# System Requirements

**Version: LibreQoS v3.** What a v3 host needs, and how to prepare it.
`libreqos-v3-preflight` checks most of this for you; see
[Installing LibreQoS v3](install.md).

LibreQoS v3 runs on a dedicated physical server or in a VM.

## Operating system

- Ubuntu Server 26.04 LTS (recommended). 24.04 LTS is still supported; the
  preflight blocks anything older. See
  [Install Ubuntu Server](ubuntu-server.md).
- Root or sudo access.
- About 50 GB of disk.

## CPU

The DPDK bridge runs one worker per CPU core, and each worker polls its NIC
queues continuously. Setup keeps two physical cores for the OS, `lqosd` and the
bridge's control threads, and uses the rest for workers:

`workers = min(physical cores − 2, NIC queues, target for the link rate)`

- **More physical cores mean more workers** and more capacity. Setup counts
  physical cores only; hyper-threads (SMT) are not used as workers.
- **Single-thread performance matters.** Each worker shapes the circuits placed
  on it, so a faster core carries more traffic per worker.
- **Disable Hyper-Threading / SMT** in the BIOS/UEFI (recommended). A worker
  polls without pause, and a busy sibling thread on the same core slows it
  down.
  - AMD: usually `Advanced → AMD CBS → CPU Common Options → Thread Enablement →
    SMT Control`, set to `Disabled`.
  - Intel: the "Hyper-Threading" option, for example on HP servers under
    `System Configuration → BIOS/Platform Configuration (RBSU) → Processor
    Options`.

Throughput depends on the number of active circuits as well as the hardware;
quote the two together. See [Known limitations](known-limitations.md).

## Memory

- The bridge needs **3072 × 2 MiB hugepages (6 GiB)**. The installer reserves
  them; if it cannot, add `default_hugepagesz=2M hugepages=3072` to the kernel
  command line and reboot. Preflight blocks when the pages are neither reserved
  nor free to reserve.
- Add memory for the OS, `lqosd` and the WebUI on top of the hugepages.

## Network interfaces

- **A management interface**, separate from the dataplane, usually the
  on-board Ethernet port. The WebUI and SSH use it.
- **Two dataplane ports** for an inline box, or one for an on-a-stick trunk.
  Setup hands them to DPDK. They must have no IP address and no bridge, bond
  or VLAN on top of them.
- **Bonded dataplane interfaces are not supported.**
- **AF_XDP is not supported.** v3 uses DPDK on the NICs below.

Supported dataplane NIC families (the drivers preflight accepts):

| Driver | NIC family | Examples |
|---|---|---|
| `mlx5_core` | NVIDIA/Mellanox ConnectX-4 Lx and newer | ConnectX-4 Lx, ConnectX-5, ConnectX-6, ConnectX-7 |
| `i40e` | Intel 700 series | X710, XL710, XXV710 |
| `ice` | Intel 800 series | E810 |
| `ixgbe` | Intel 10G | X520, X540, X550 |
| `igb` | Intel 1G | I350 |

Intel often vendor-locks SFP+ modules; check module compatibility before you
buy. Mellanox cards do not have this problem.

The NIC needs at least as many RX/TX queues as the workers you want; a NIC with
fewer queues limits the worker count.

Other NICs may work but are not supported. If you test one successfully, please
tell us at support [at] libreqos.io.

## Virtual machines

- Use the modern virtio NIC (`1af4:1041`) with `disable-legacy=on` on both
  dataplane NICs, or pass a supported NIC through to the VM with PCI
  passthrough.
- Give the VM enough vCPUs for the workers plus the two reserved cores, and
  memory for the hugepages.
- **Proxmox:** enable [Multiqueue](https://forum.proxmox.com/threads/where-is-multiqueue.146783/)
  on the dataplane NICs, set to the number of vCPUs.

## Measured capacity

One reference host, measured in the LibreQoS lab with a TRex traffic generator.
These are single 30-second runs per rate, not a guarantee; use them as a
starting point and leave headroom.

**Host:** AMD Ryzen 9 7950X (16 cores), 14 workers, NVIDIA ConnectX (`mlx5`)
dataplane NIC, the default `balanced` power profile (turbo off, 4.5 GHz),
1,000 circuits, CAKE on every circuit.

**Production-like traffic.** The packet sizes and the upload/download mix
come from a production ISP network: download frames of 1,291 bytes, upload
frames of 386 bytes, and about 0.4 upload packets per download packet.

| Download offered | Total delivered (both directions) | Packets/s (total) | Download loss | Upload loss |
|---:|---:|---:|---:|---:|
| 25 Gbps | 28.0 Gbps | 3.4 M | 0.007 % | 0.001 % |
| 40 Gbps | 44.8 Gbps | 5.4 M | 0.03 % | < 0.001 % |
| 50 Gbps¹ | 53.4 Gbps | 6.8 M | 4.9 % | 2.6 % |

¹ 15-second run. At this rate the host is past capacity.

So this host carries about **45 Gbps of production-like traffic with
negligible loss**, about **3.2 Gbps per worker**.

**Packet rate sets the limit, not bit rate.** This host is clean at 5.4
million packets per second and past capacity at 6.8 million. If your average
packet is smaller than this mix, expect fewer Gbps: estimate your peak packet
rate (total bits per second ÷ 8 ÷ average packet size) and compare that.

**The power profile matters.** With the `eco` profile (frequency capped at
65 %), the same host lost 16.6 % of download traffic at 40 Gbps where
`balanced` lost 0.03 %. See [CPU power profile](power.md).

## Example hardware

These servers use NICs from the supported list. Per-model v3 throughput has
not been measured; the figures above are from the reference host only.

| Class | Model | CPU | NIC |
|---|---|---|---|
| Small form factor | Minisforum MS-01 | i9-12900H | built-in Intel X710 (2 × SFP+) |
| 1U | Supermicro SYS-511R-M | Xeon E-2488 | Intel X710-BM2 (2 × SFP+) |
| 1U | Dell PowerEdge R260 | Xeon E-2456 | Intel X710-T2L (2 × 10G RJ45) |
| 1U | Supermicro AS-1115S-FWTRT | EPYC 8534P | Mellanox (2 × SFP28) |
| 1U | Supermicro AS-1015SV-WTNRT | EPYC 8534P | Mellanox (2 × QSFP56) |
| 2U | Supermicro AS-2015CS-TNR | EPYC 9745 | Mellanox MCX653106A-HDAT |

On the MS-01, replacing the thermal paste lowers CPU temperatures noticeably
(this may void the warranty). A used server from a reputable vendor is also a
cost-effective option.
