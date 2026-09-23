# Debian 13 Support Plan

Status: complete on branch `debian13_support` (awaiting merge)

## Goal

Run LibreQoS v2.2 on Debian 13 ("trixie") with the same support level the
Ubuntu 24.04 path has today: buildable from source, installable as a `.deb`,
and covered by the disposable RADIUS VM harness.

## Deliverables

1. `src/build_prereqs.sh` — shared, distro-aware build prerequisite helper.
2. `src/build_rust_debian.sh` — Debian front-end for `build_rust.sh`.
3. `src/build_pkg_debian.sh` — Debian front-end for `build_dpkg.sh`.
4. `src/build_dpkg.sh` — runtime dependency fixes for Debian minimal installs.
5. `radius-harness` — `LAB_GUEST_OS=debian` guest support with a pinned trixie image.
6. Documentation updates (developer docs updated; operator-facing support statements remain a product decision).

## Design decisions

- One implementation, two front-ends. `build_rust.sh` and `build_dpkg.sh` remain the
  source of truth; the Debian scripts export `LQOS_DISTRO=debian` and delegate. This
  keeps the paths from drifting, which AGENTS.md and the packaging skill call out as
  a recurring failure mode.
- Distro detection is a fallback, not a requirement. `LQOS_DISTRO` overrides
  `/etc/os-release`, so the Debian scripts work on derivatives and on hosts where
  detection is ambiguous.
- The packaging script is distro-neutral once the dependency fix lands. The Debian
  front-end pins the flavor, installs Debian prerequisites, and serves as the
  documented packaging entry point for Debian hosts.
- Harness guest OS is a variable defaulting to Ubuntu, so the existing workflow does
  not change.

## Compatibility fixes in scope

### Build (currently broken on Debian)

| Problem | Fix |
| --- | --- |
| `linux-tools-common` and `linux-tools-$(uname -r)` do not exist in Debian, and `apt install` aborts the whole transaction when a name is unknown | Debian list installs `bpftool` (required by `lqos_sys/build.rs`) and `linux-perf`; the kernel tools are installed best-effort so an unavailable `linux-tools-<kernel>` cannot abort the common set |
| Debian installs `bpftool` in `/usr/sbin`, which is not on the non-root PATH there, and `lqos_sys` invokes `bpftool` by name | Prepend `/usr/sbin` to PATH in the prerequisite helper |
| `make` is a hard requirement of the vendored libbpf build and is not in the Ubuntu list | Add it to both distro lists (`pkg-config` was already present) |
| `apt install` without `-y` in a scripted path, and no `apt-get update` before a fresh-host install | Use `apt-get update` plus `apt-get install -y` |

### Packaging

| Problem | Fix |
| --- | --- |
| `lqosd` directly needs `libelf.so.1`, `libssl.so.3`/`libcrypto.so.3`, and `libz.so.1`; a minimal Debian install may lack `libelf1t64` | Add `libelf1t64, libssl3t64, zlib1g` to `Depends` (`libzstd1` arrives through `libelf1t64`) |
| `lqos_setup` network apply requires netplan; Ubuntu ships it, Debian does not | `Recommends: netplan.io, ethtool`; `lqosd` also uses `ethtool` for offload and coalescing tuning, with non-fatal failures |

### Checked against Debian 13 package and kernel data (no changes needed)

- Debian 13 kernel (cloud and generic): `CONFIG_DEBUG_INFO_BTF=y` for CO-RE, BPF/JIT/XDP/TC-BPF, `sch_cake=m`, veth/bridge/VLAN/virtio-net.
- The cloud kernel flavour disables `CONFIG_PPP`, so the harness pins the generic image; the generic kernel ships `CONFIG_PPP=m` and `CONFIG_PPPOE=m` for the PPPoE client.
- FreeRADIUS 3.2.7 still uses `/etc/freeradius/3.0`, so harness paths are unchanged.
- `liblqos_python.so` is PyO3 `abi3-py310` and loads under Debian's Python 3.13; no removed-stdlib usage in `src/*.py`.
- The Ubuntu systemd hotfix is inert on Debian (`is_supported_os` guard) and does not block the package postinst.
- Host-built binaries run against trixie's glibc 2.41 and OpenSSL 3.5.

## Phases

- [x] Phase 0: branch and this plan.
- [x] Phase 1: shared prerequisite helper and `build_rust_debian.sh`.
- [x] Phase 2: `build_dpkg.sh` dependency fixes and `build_pkg_debian.sh`.
- [x] Phase 3: harness Debian guest support (`lab.env`, `lab`, `user-data.yaml`, README).
- [x] Phase 4: static validation (`bash -n` on touched scripts, harness smoke checks, review-agent findings applied).
- [x] Phase 5: VM validation: `lab init` / `up` / `configure` / `test` with `LAB_GUEST_OS=debian`, including the guest-OS and BPF-map assertions.
- [x] Phase 6: validate the built `.deb` inside the Debian guest (`lab check-package`).
- [x] Phase 7: developer docs updated to stop duplicating the apt list; the `bash -n` CI gate is deferred to a separate change.
- [x] Phase 8: final review pass (heckler, reaper, thomas, beck, jonas) and merge gate.

## Validation results

- `bash -n` passes for every touched script.
- Debian 13 VM run (generic image, kernel `6.12.107+deb13-amd64`): all three RADIUS lifecycle cases passed, including dynamic-circuit create, Interim-Update retention, and removal.
- The guest-OS assertion, the BPF map-pinning assertion, and `lab check-package` (`.deb` dependency resolution) all passed on Debian 13.
- The Debian prerequisite package set resolves on trixie, and `/usr/sbin` is absent from the default non-root PATH, confirming the `bpftool` PATH fix.
- Ubuntu 24.04 regression run: the same three lifecycle cases passed with the shared harness changes, and lease resolution ignored two generations of stale leases.

## Known gaps

- `build_rust_debian.sh` and `build_pkg_debian.sh` were not executed on a Debian host end-to-end. The harness builds the runtime bundle on the Ubuntu host; the Debian package list and PATH fixes were validated against trixie package data and the Debian guest.
- The `.deb` postinst was not exercised on Debian; `lab check-package` simulates dependency resolution only.
- Repo bug found while testing, outside this branch's scope: `maybe_migrate_uisp_capacity_defaults` writes a `[uisp_integration]` table without the required `enable_uisp` field, so any config lacking that section fails to parse after migration. The harness fixture was updated to the current schema; the migration itself still needs a fix.

## Validation checklist

- `bash -n` passes for every touched script.
- `LQOS_DISTRO` detection returns `ubuntu` on this host and `debian` under the override.
- The harness image pin verifies against Debian's `SHA512SUMS`.
- The Debian guest reaches the same three RADIUS lifecycle assertions as Ubuntu.
- `lqosd` starts on the Debian guest and pins maps under `/sys/fs/bpf`.
- The default Ubuntu harness path passes the same lifecycle test with the shared changes (`LAB_GUEST_OS=ubuntu`).

## Risks and open questions

- Host `osinfo-db` may not know `debian13`; the `LAB_OS_VARIANT` override covers that.
- The RouterOS console password step is interactive; the VM run needs an operator at `lab console` once.
- Whether to declare Debian 13 supported in operator-facing docs is a product decision. This branch proves the technical path first.

## Accepted review decisions

- `netplan.io` stays in `Recommends` rather than `Depends`: runtime shaping does not need it, the setup flow does, and default apt installs recommends. Debian 13's cloud image ships it.
- The `.deb` keeps the time64 dependency names (`libelf1t64`, `libssl3t64`). That matches the supported Ubuntu 24.04+ and Debian 13 targets; older releases are out of scope.
- `libzstd1` stays out of `Depends`: `lqosd` reaches it through `libelf1t64`, which declares it. Generating `Depends` with `dpkg-shlibdeps` is the follow-up that would catch future direct libraries.
- The build scripts keep assuming `sudo`, matching `build_rust.sh` and `update_api.sh`. Minimal installs without `sudo` are out of scope for this branch.
- `management_ip` can still pick a previous lab's *unexpired* lease before the new guest requests DHCP, and it ignores leases libvirt reports as unlimited. The documented flow includes a console step between `up` and `configure`, so these are recorded rather than reworked.
- Developer docs reference `build_rust_debian.sh`; operator-facing requirements pages still state Ubuntu 24.04 as the supported OS until that product decision is made.
