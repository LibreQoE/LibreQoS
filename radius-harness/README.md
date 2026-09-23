# LibreQoS RADIUS PPPoE harness

This disposable libvirt harness verifies the RADIUS accounting lifecycle end to
end: a RouterOS CHR NAS authenticates a PPPoE client with FreeRADIUS, sends
accounting to a root-run LibreQoS guest, and the test verifies dynamic-circuit
creation, profile selection, Interim-Update, and removal on disconnect.

The Linux guests default to Ubuntu 24.04. Set `LAB_GUEST_OS=debian` to run them
on Debian 13 instead; the guest image and its checksum are pinned in
[lab.env](lab.env).

It creates `radius-*` libvirt domains and networks, sparse qcow2 overlays, and
temporary `vbr-r*` host-side bridge interfaces for its isolated networks.
`./radius-harness/lab down` removes them with the overlays and VMs. `purge`
additionally removes downloaded base images and the host-built runtime bundle.
The harness does not change existing host interfaces, LibreQoS services, or
host shaping.

The `radius-*` domain and network names are reserved for this harness. Do not
run `down` or `purge` on a host where another workload intentionally uses one
of those names.

## Requirements

Run from a LibreQoS checkout on a host with libvirt/KVM and OVMF/edk2 UEFI
firmware. Install `virsh`, `virt-install`, `qemu-img`, `cloud-localds` (from
`cloud-image-utils`), `curl`, `unzip`, `rsync`, `sshpass`, and a Rust toolchain.
The invoking account must be allowed to manage the configured libvirt URI
(default: `qemu:///system`).

The harness discovers common OVMF locations. If your distribution stores its
firmware elsewhere, set `OVMF_CODE_PATH` and `OVMF_VARS_PATH` in the environment
before `up`.

The pinned image hashes are in [lab.env](lab.env). Review and update the URL,
version, and SHA-256 together when changing an image. Debian publishes SHA-512
digests in `SHA512SUMS`, so the Debian image pins a SHA-512 value instead.

The harness builds the LibreQoS runtime bundle on the host and copies it into
the guest. The host must produce binaries whose glibc symbol versions the guest
provides; a host far newer than the guest can produce a bundle the guest cannot
start.

## Run

Set the one-time RouterOS administrator password and the lab-only RADIUS shared
secret. They are never committed and are written only to ignored run-state and
disposable guests.

```bash
export ROUTEROS_ADMIN_PASSWORD='a-routeros-password'
export RADIUS_SHARED_SECRET='a-lab-radius-secret'

./radius-harness/lab init
./radius-harness/lab up
./radius-harness/lab console       # set the RouterOS admin password once; exit with Ctrl+]
./radius-harness/lab configure
./radius-harness/lab test
./radius-harness/lab down
```

`init` downloads the pinned guest and RouterOS bases once and builds `lqosd`
and `lqos_python` on the host. The guest receives only the resulting runtime
bundle, not the Rust build tree.

To run the Linux guests on Debian 13 instead of Ubuntu, set
`LAB_GUEST_OS=debian` before `init`; later commands reuse the recorded choice.
The Debian pin uses the generic image rather than genericcloud because the
cloud kernel disables PPP, which the PPPoE client needs. If the host osinfo
database does not know `debian13`, set `LAB_OS_VARIANT=debian12` (or `generic`)
before `up` as well.

The test covers three cases:

1. A RADIUS rate-limit attribute supplies the queue speed.
2. A known PPPoE username in the `MAC` field of `ShapedDevices.csv` supplies
   the circuit identity and speed when RADIUS supplies no rate.
3. An unknown username receives the configured fallback queue.

The harness uses temporary IPv4 ranges `198.18.10.0/24`, `198.18.30.0/24`, and
`100.64.0.0/16`. If those conflict with local infrastructure, change the
templates before running it.

## Commands

```text
./radius-harness/lab init       Download bases and build the runtime bundle.
./radius-harness/lab up         Create networks, overlays, and VMs.
./radius-harness/lab status     Show domains and management addresses.
./radius-harness/lab configure  Install fixtures and start services.
./radius-harness/lab test       Run the lifecycle assertions.
./radius-harness/lab check-package  Check .deb dependency resolution in the guest.
./radius-harness/lab down       Remove VMs, networks, overlays, and secrets.
./radius-harness/lab purge      Also remove cached images and runtime artifacts.
./radius-harness/lab console    Open the RouterOS serial console.
```
