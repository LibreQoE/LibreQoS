# Installing LibreQoS v3

**Version: LibreQoS v3 beta 12 (pre-release).** This page is the operator
install path. The exact install targets and file modes are in
`packaging/INSTALL.md` in the source tree; this page is the end-to-end summary.

```{note}
Beta 12: auto-CAKE is off by default and AF_XDP is not supported. Read
[Known limitations](known-limitations.md) before installing.
```

## 1. Add the LibreQoS apt repo

LibreQoS v3 ships from the LibreQoS apt archive, and the control-plane `postinst`
fetches the separate `libreqos-dataplane` package from the **same** archive — so
the repo must be configured **before** `apt install libreqos`.

```bash
curl -fsSL https://packages.libreqos.com/keys/libreqos-packages-keyring.gpg \
  | sudo tee /usr/share/keyrings/libreqos-packages-keyring.gpg >/dev/null

echo "deb [signed-by=/usr/share/keyrings/libreqos-packages-keyring.gpg] https://packages.libreqos.com beta main" \
  | sudo tee /etc/apt/sources.list.d/libreqos.list

sudo apt-get update
```

**Channel:** the apt suite is the release channel.

| Suite | Carries | Use it if |
|---|---|---|
| `stable` | GA releases (`vX.Y.Z` tags, `master`) | production |
| `beta` | release candidates (`vX.Y.Z-betaN` tags) | you want early access |
| `nightly` | the `develop` tip, built on every push | you are testing; expect breakage |

Use **`beta`** for the current v3 release candidate; **`stable` opens at GA**.
Installing from an empty suite resolves nothing, and the postinst's dataplane
fetch then fails closed with `NOT FETCHED`. At GA, change the suite word in
`/etc/apt/sources.list.d/libreqos.list` to `stable` and re-run `apt-get update`.

**Already on LibreQoS v2?** You have the **legacy** `repo.libreqos.com` archive
(a systemd-hotfix entry + its key). It does **not** carry v3 — see
[Upgrading from v2.0](upgrade-from-v2.md) for the transition.

## 2. Run the preflight before installing anything else

The v3 `libreqos` package's `preinst` **stops the old LibreQoS daemons** as part
of the upgrade, then runs the preflight checker and refuses the install on a
**BLOCK**. Run the checker first, so the verdict arrives before anything on the
host changes.

The checker ships as the tiny `libreqos-preflight` package — a static binary with
no daemons and no service actions. Install only that package, then run it:

```bash
sudo apt install libreqos-preflight    # the checker only; no daemons are touched
libreqos-v3-preflight
```

It reports **PASS / WARN / BLOCK** and answers three questions up front:

- **Is the hardware supported?** (the mlx5/i40e NIC class, the pair, hugepages,
  kernel, CPU ISA).
- **Will my network compile?** — the v3 compiler dry-runs your existing
  `network.json` + `ShapedDevices.csv`.
- **Would I exceed the licensed circuit count?** (beyond-cap subscribers are
  left unshaped under v3).

A **BLOCK** means the upgrade is not for this host (or not yet) — **do not
install `libreqos`**. A **WARN** may proceed. The `libreqos` `preinst` runs the
same checker and refuses a **BLOCK**; the explicit override is
`LQOS_PREFLIGHT_FORCE=1` (logged loudly). If an install is refused at the
preinst, the old daemons have already been stopped — restart them and fix the
block before retrying.

## 3. Host requirements

- **Ubuntu Server 24.04 LTS (Noble) or newer.** LibreQoS v3 is built against
  **glibc 2.39**; an older host would install and then fail with
  `GLIBC_2.39 not found`. The preflight blocks this.
- **Two dataplane NICs** of a supported class (a single-port card needs a second
  card for an inline box).
- **Hugepages.** The installer attempts a runtime 2M allocation (default
  `3072`) so the first boot can shape without a reboot. If that fails, add
  `default_hugepagesz=2M hugepages=3072` (or the 1G posture for large mempools)
  to the kernel command line and reboot. The setup check **WARNs** (never
  blocks) when pages are absent.
- **Management on a separate NIC.** A DPDK dataplane port must have no
  global-scope address and no bridge/bond/VLAN upper. The port preflight
  re-checks this at every bridge start and fails closed.
- **No vfio-pci enablement for the CX-5 class** — the bridge attaches
  verbs-in-place and keeps the kernel netdev.

### Proxmox / VM (virtio) deployments

LibreQoS v3 supports Proxmox VE **8 or 9**; **PVE 9 is not required**.

- The VM dataplane is the DPDK **`net_virtio`** PMD, bound with
  `uio_pci_generic` (`new_id 1af4 1041`) and unbound from `virtio-pci`.
- It requires the **modern virtio 1.0 device, PCI ID `1af4:1041`**. A **default
  Proxmox `virtio` NIC is the transitional `1af4:1000` and is not bound** by the
  packaged installer. Set `disable-legacy=on` on the two dataplane NICs — a QEMU
  property available since QEMU 2.7, so PVE 8 supports it.
- Verify the dataplane NICs: `lspci -nn | grep 1af4:1041` (both must match).
  `1af4:1000` is the default/transitional device, not the dataplane one.
- Multiqueue must equal the vCPU count; `--rx-queues`/`--tx-queues` must match
  the NIC queue count.
- Do **not** enable `iommu_platform=on` + `virtio-iommu-pci` for the
  `net_virtio` path (userspace PCI cannot drive `iommu_platform` virtio; that is
  the retired AF_XDP posture). Do not switch the VM to q35 — it reshuffles NIC
  names; bind netplan by MAC.

## 4. Install the control plane and the dataplane

The v3 release ships as **two artifacts** (the GPLv2 distribution split,
`docs/distribution_split_card.md`):

- **`libreqos` — the control plane.** `lqosd`, the scheduler integration hub, the
  first-run setup, the power helper, the web UI, and the optional API. It
  `Pre-Depends:` on **`libreqos-preflight`** and `Depends:` on **`python3-pip,
  python3-venv, nano, curl, unzip, ca-certificates`** — it is **not** a
  zero-dependency package.
- **`libreqos-dataplane` — the DPDK bridge.** A separate GPLv2 artifact
  distributed from the LibreQoS apt repo (`packages.libreqos.com`): the
  `bridge_rs` binary, its DPDK runtime closure, the EAL PMDs, the packaged bind
  helper, the bridge units/wrapper, the sudoers and logind grants, and the
  GPLv2 §3(b) source-on-request offer. The control-plane `postinst` fetches it
  via `update_bridge.sh` from the same archive (**§1** must be configured
  first); an unreachable repo prints **NOT FETCHED** and exits 0 (fail-closed:
  the control plane runs, the bridge stays inactive).

The canonical units:

- `libreqos-setup` (oneshot verify; the bridge `Requires=` it)
- `libreqos-lqosd`
- `libreqos-bridge` (from the dataplane artifact)
- `libreqos-scheduler`
- `libreqos-power` (power-profile helper)
- `libreqos-api` (enabled on a successful install-time fetch)

```bash
sudo apt install libreqos          # control plane (pulls libreqos-preflight + deps)
# the postinst fetches libreqos-dataplane; or run it explicitly:
sudo /opt/libreqos/src/update_bridge.sh
sudo editor /etc/libreqos/bridge.env     # bridge paths + EAL args only
sudo editor /etc/libreqos/lqos.conf      # the SINGLE source: the [dataplane] rate + the world
```

The shaping rate is the operator's, in **`/etc/libreqos/lqos.conf`**. The
`[dataplane]` section is authoritative (f64, per direction):

```toml
[dataplane]
download_mbps = 10000.0
upload_mbps = 10000.0
```

The bridge resolves the rate **per direction** as
`[dataplane] download_mbps`/`upload_mbps` → `[queues] downlink_bandwidth_mbps`/
`uplink_bandwidth_mbps` (the v2 transition fallback) →
`LIBREQOS_DPDK_DOWNLOAD_MBPS`/`LIBREQOS_DPDK_UPLOAD_MBPS` → default `1000`. The
typed `bridge.conf bandwidth` source is **removed**; a stale `bridge.conf`
carrying a `bandwidth` block now makes the bridge **fail loud**
(`deny_unknown_fields`) rather than silently overriding `lqos.conf`.
`bridge.env` carries only the config/LKG paths and the EAL args.

(Source of truth: the control plane `bandwidth_mbps()` in
`src/src/rust/lqos_config/src/etc/v15/top_config.rs` and the bridge
`download_mbps()` in `rust/libreqos_dpdk/bridge_rs/src/config.rs`; see also
`docs/lqos_conf_single_source_card.md`.)

The DPDK runtime closure and the bridge install under `/usr/libexec/libreqos/`;
the web UI bundles install to `/opt/libreqos/src/bin/static2` (build with
`STATIC2_DIR=`; without them `lqosd` serves no dashboard — the dataplane is
unaffected).

## 5. Start in the right order

`libreqos-bridge` `Requires=` the oneshot verify unit (`libreqos-setup.service`),
so a failed preflight blocks the bridge loudly. The packaged units carry the boot
order (**lqosd-first**):

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now libreqos-setup.service   # oneshot verify
sudo systemctl start libreqos-lqosd.service
sudo systemctl start libreqos-bridge.service
```

The **first-run gate** is the separate `libreqos-first-run.service` (enabled by
the postinst's `activate-setup`); complete `lqos_setup tui` (NIC pick/verify →
units → `activate-runtime`) to activate the runtime units.

On first boot the bridge starts from the last-known-good transaction in
`/var/lib/libreqos-bridge/bridge_lkg.json` (persistent; the code default
`/run` is tmpfs and would be wiped at reboot).

## 6. MANDATORY host protections

- **`/etc/sudoers.d/libreqos-bridge`** must be `0440 root:root`. It is the only
  privilege grant: three verbs (`start|stop|restart`) on the one unit, for the
  service user. Validate with `visudo -cf /etc/sudoers.d/libreqos-bridge`.
- **`logind/50-libreqos-ipc.conf`** → `/etc/systemd/logind.conf.d/`, then
  `systemctl restart systemd-logind`. Without it, systemd's `RemoveIPC` default
  deletes the control segment when the operator's last login ends (a channel
  blackout until the bridge restarts).

## 7. Dataplane bind and first verification

- The dataplane artifact ships the packaged bind (`libreqos-bridge-bind.service`
  → `libreqos-bind-dpdk-ports.sh`), which resolves the WAN/LAN BDFs and binds
  them idempotently, failing closed with **named errors**. The bind is **refused
  while a NIC is still enslaved by a netplan bridge** — go back to the adoption
  step in [Upgrading from v2.0](upgrade-from-v2.md).
- Confirm the seam and the units — see [Operating v3](operate.md) and
  [Troubleshooting](troubleshooting.md).

## Deploy rule: the full matched set

A v3 `lqosd`/control change ships in the **control** package; a `bridge_rs`
change ships in the **dataplane** package. When deploying, install the **full
matched set** (`component=all`) together, stop the bridge first, and run
`dpkg --configure -a`. Do not mix a control-only candidate with an older
dataplane.
