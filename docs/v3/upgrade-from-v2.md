# Upgrading from LibreQoS v2.0 to v3

**Version: LibreQoS v3 beta 12 (pre-release).** This is the operator entry point
for a v2.0 → v3 upgrade. The **consolidated, step-by-step guide** ships in the
source tree at `packaging/UPGRADE.md`; this page is its summary.

```{note}
The canonical consolidated guide is `packaging/UPGRADE.md`. This page mirrors its
shape; where they differ, the packaging guide is authoritative.
```

```{note}
Beta 12 is a pre-release. Keep a v2.0 rollback image/configuration and the
[v2.0 documentation archive](../v2.0/index.md) available. Read
[Known limitations](known-limitations.md) before upgrading.
```

## The path, in order

0. **Add the new apt repo** — you are on the **legacy** `repo.libreqos.com`
   archive, which does not carry v3 (its `beta` is frozen and its `stable` is
   absent). Add the key + a **`beta`** sources entry for
   `packages.libreqos.com` (see [Installing v3 §1](install.md)) **before**
   installing — `beta` carries the current release candidate, and the suite
   word becomes `stable` at GA; if an existing
   `/etc/apt/sources.list.d/libreqos.list` points at
   the legacy host, **replace** it. The v2 systemd-hotfix entry
   (`libreqos-systemd-hotfix.list`) and its key
   `/usr/share/keyrings/libreqos-archive-keyring.gpg` are **independent** — keep
   the key; the rollback (step 8) needs the legacy `archive` suite.
1. **Preflight — before installing the packages.** The v3 `libreqos` `preinst`
   **stops the old daemons** and then runs the checker, refusing the install on
   a **BLOCK**. Run the checker first: install only the static checker package
   (`sudo apt install libreqos-preflight` — no daemons, no service actions),
   then run `libreqos-v3-preflight`. Read PASS/WARN/BLOCK; a **BLOCK** means
   stop here. Watch the netplan adoption line and any `OPERATOR-CONSOLE
   REQUIRED`.
2. **Install LibreQoS v3** — the control plane + the separate dataplane artifact
   (see [Installing v3](install.md)); `lqos_setup` runs the `bridge.conf`
   migration and `lqosd` migrates the runtime state to schema 3.
3. **Adopt the Linux bridge** (netplan) — transactional; note the **backup id**;
   verify management still answers; never proceed remotely without console access
   when management rides the bridge.
4. **Bind the dataplane ports** — the packaged bind
   (`libreqos-bridge-bind.service`); refused while a NIC is still enslaved by a
   netplan bridge.
5. **Migrate the configuration** — the key facts are below.
6. **LKG** — the last-known-good transaction survives the purge; confirm the
   persistent path.
7. **Verify** — the units active, the seam healthy (`lqosd --print-seam-state`),
   the expected generation applied.
8. **Roll back** if needed — restore the adoption backup first, then the
   packaged `/usr/sbin/libreqos-v3-rollback` (also
   `/usr/libexec/libreqos/libreqos-v3-rollback`). `--from-archive <v2>` resolves
   the retained v2 package
   from the **`archive` suite on the LEGACY `repo.libreqos.com`**
   (`packages.libreqos.com` serves no `archive`), so keep the legacy source entry
   and its `libreqos-archive-keyring.gpg` until the rollback window closes.

## Configuration-migration facts

- **`bridge.conf`** is strict (`deny_unknown_fields`); setup removes the legacy
  `circuit_capacity` key automatically. A hand-edited/unparsable file is left
  alone and fails **loud**.
- **Bandwidth** is single-sourced in `lqos.conf [dataplane] download_mbps` /
  `upload_mbps` (f64, per direction). Resolution: `[dataplane]` → `[queues]
  downlink/uplink_bandwidth_mbps` (the v2 transition fallback, so the v2 keys
  carry over) → `LIBREQOS_DPDK_DOWNLOAD_MBPS`/`LIBREQOS_DPDK_UPLOAD_MBPS` →
  default `1000`. The typed `bridge.conf bandwidth` source is **removed** — a
  stale one now makes the bridge **fail loud** (`deny_unknown_fields`); it no
  longer silently outranks `lqos.conf`. The `lqos_setup` activation **folds** the
  rate into `[dataplane]`, retires the duplicate `bridge.env`
  `DOWNLOAD/UPLOAD_MBPS` keys, and leaves a `.pre-s7.bak` backup.
- **Retired v2.0 keys** (ignored, not fatal): `[queues] fast_queues_fq_codel`,
  `queue_check_period_ms`, `[stormguard] circuit_fallback_persist`, and the dead
  `[treeguard]` block. `[tuning]` is relabelled "Management NIC / Kernel Netdev
  only".
- **UISP interval** migrates `long_term_stats.uisp_reporting_interval_seconds` →
  `integration_common.queue_refresh_interval_mins` (`seconds / 60`, rounded,
  clamped `>= 1`; only when the python-derived poll is still the default).
- **`do_not_track_subnets`** is restored: a per-subnet opt-out that suppresses
  flows **and** RTT (and is honoured by hosted Insight).

> Source of truth: `docs/lqos_conf_single_source_card.md`; the control plane
> `bandwidth_mbps()` in `src/src/rust/lqos_config/src/etc/v15/top_config.rs`,
> the bridge `download_mbps()` in `rust/libreqos_dpdk/bridge_rs/src/config.rs`,
> and the §7 fold in `src/src/rust/lqos_setup/src/config_migrate.rs`.
