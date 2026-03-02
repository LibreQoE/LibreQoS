# Recipe: Proxmox VM Deployment with 3 NICs

Use this pattern for VM-based deployments where LibreQoS runs on Ubuntu Server 24.04 with dedicated management and shaping interfaces.

## Fit

- Best for: operators standardizing on Proxmox where VM operations are already mature.
- Avoid when: target throughput and latency goals require bare-metal headroom.

## Pattern Selection

Pattern A is the default and recommended layout for clarity.

- Pattern A (recommended): dedicated Proxmox bridges for shaping (`vmbr1`, `vmbr2`)
- Pattern B (alternative): single Proxmox bridge (`vmbr0`) with VLAN-tagged virtio NICs

## Interface Roles

- `ens18`: management interface (IP assigned)
- `ens19`: shaping port 1
- `ens20`: shaping port 2

`ens19` and `ens20` are the two shaping interfaces used by LibreQoS (`to_internet`/`to_network`).
`ens18`/`ens19`/`ens20` are example names; verify actual guest interface names before setting `to_internet` and `to_network`.

## Pattern A (Recommended): Dedicated Shaping Bridges

Use this when you want the cleanest operational model.

Host-side intent:

- `vmbr1` is backed by shaping interface A path on the Proxmox host.
- `vmbr2` is backed by shaping interface B path on the Proxmox host.
- Management is usually untagged on `vmbr0`.

Host-to-guest mapping reference:

| Proxmox host port/path | Bridge | VM vNIC | Guest NIC | LibreQoS role |
|---|---|---|---|---|
| Management uplink (example `eno1`) | `vmbr0` | `net0` | `ens18` | Management |
| Shaping path A (example `eno2`) | `vmbr1` | `net1` | `ens19` | `to_internet` or `to_network` |
| Shaping path B (example `eno3`) | `vmbr2` | `net2` | `ens20` | opposite of `ens19` |

VM Hardware (Proxmox UI) example:

- `net0`: `virtio`, `bridge=vmbr0` (management)
- `net1`: `virtio`, `bridge=vmbr1`, `multiqueue=<vCPU count>` (shaping A)
- `net2`: `virtio`, `bridge=vmbr2`, `multiqueue=<vCPU count>` (shaping B)

In-guest mapping:

- `ens18 -> net0 -> vmbr0` (management)
- `ens19 -> net1 -> vmbr1` (shaping A)
- `ens20 -> net2 -> vmbr2` (shaping B)

```{mermaid}
flowchart LR
    subgraph HOST[Proxmox Host]
        H0[vmbr0 (mgmt untagged)]
        H1[vmbr1 (shaping A path)]
        H2[vmbr2 (shaping B path)]
    end

    subgraph VM[LibreQoS VM]
        N0[net0 virtio]
        N1[net1 virtio]
        N2[net2 virtio]
        E18[ens18 mgmt]
        E19[ens19 shaping A]
        E20[ens20 shaping B]
    end

    H0 --> N0 --> E18
    H1 --> N1 --> E19
    H2 --> N2 --> E20
```

## Pattern B (Alternative): Single Bridge with VLAN-Tagged NICs

Use this when your Proxmox design standardizes on one VLAN-aware bridge.

Host-side intent:

- `vmbr0` carries management and shaping VLANs as a trunk.
- Management is often tagged in this pattern.

VM Hardware (Proxmox UI) example:

- `net0`: `virtio`, `bridge=vmbr0`, `tag=99` (management example VLAN)
- `net1`: `virtio`, `bridge=vmbr0`, `tag=110`, `multiqueue=<vCPU count>` (shaping A example VLAN)
- `net2`: `virtio`, `bridge=vmbr0`, `tag=120`, `multiqueue=<vCPU count>` (shaping B example VLAN)

Notes:

- VLAN IDs above are examples. Use IDs matching your network design.
- With Proxmox NIC tag assignment, traffic is typically presented untagged inside the guest NIC (no guest VLAN subinterface required for this model).

```{mermaid}
flowchart LR
    subgraph HOST[Proxmox Host]
        T0[vmbr0 VLAN trunk]
    end

    subgraph VM[LibreQoS VM]
        PN0[net0 virtio tag 99]
        PN1[net1 virtio tag 110]
        PN2[net2 virtio tag 120]
        PE18[ens18 mgmt]
        PE19[ens19 shaping A]
        PE20[ens20 shaping B]
    end

    T0 --> PN0 --> PE18
    T0 --> PN1 --> PE19
    T0 --> PN2 --> PE20
```

## Prerequisites

1. Review [Prerequisites](prereq.md) and [System Requirements](requirements.md).
2. Enable multiqueue on shaping vNICs; set queue count equal to VM vCPU count.
3. For throughput above 10 Gbps, use NIC passthrough where required.

## Netplan Pattern

Example baseline:

```yaml
network:
  version: 2
  ethernets:
    ens18:
      addresses:
        - 100.99.0.10/24
      routes:
        - to: default
          via: 100.99.0.1
      nameservers:
        addresses: [1.1.1.1, 8.8.8.8]
    ens19:
      dhcp4: no
      dhcp6: no
    ens20:
      dhcp4: no
      dhcp6: no
```

Then configure bridge behavior per [Configure Shaping Bridge](bridge.md).

## Validation Checklist

1. Confirm which pattern is deployed on this VM (A or B).
2. For Pattern A, confirm `net1->vmbr1` and `net2->vmbr2`.
3. For Pattern B, confirm `net1->vmbr0 tag 110` and `net2->vmbr0 tag 120` (or your chosen tags).
4. Confirm `to_internet` / `to_network` map correctly to `ens19`/`ens20`.
5. Confirm `ens19` and `ens20` have no IP assignment in guest Netplan.
6. Confirm scheduler and daemon are healthy.
7. Confirm throughput and latency match expected VM envelope.
8. Confirm no asymmetric shaping behavior (especially in on-a-stick variants).

## Rollback

1. Move path back to previous shaper or bypass route.
2. Revert VM interface or queue settings.
3. Restart services and re-validate.

## Related Pages

- [Prerequisites](prereq.md)
- [System Requirements](requirements.md)
- [Configure Shaping Bridge](bridge.md)
- [Troubleshooting](troubleshooting.md)
