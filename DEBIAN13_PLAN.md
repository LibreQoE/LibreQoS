# Debian 13 Support Plan

Status: in progress (branch `debian13_support`)

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
6. Documentation updates (contributor docs now; operator docs after the VM run passes).

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
| `linux-tools-common` and `linux-tools-$(uname -r)` do not exist in Debian, and `apt install` aborts the whole transaction when a name is unknown | Debian list installs `bpftool` (required by `lqos_sys/build.rs`) and `linux-perf` |
| `make` and `pkg-config` are hard requirements of the vendored libbpf build and are not in the Ubuntu list | Add them to both distro lists |
| `apt install` without `-y` in a scripted path | Use `apt-get install -y` |

### Packaging

| Problem | Fix |
| --- | --- |
| `lqosd` links `libelf.so.1`, `libssl.so.3`, `libzstd.so.1`, `libz.so.1`; a minimal Debian install may lack `libelf1t64` | Add `libelf1t64, libssl3t64, libzstd1, zlib1g` to `Depends` |
| `lqos_setup` network apply requires netplan; Ubuntu ships it, Debian does not | `Recommends: netplan.io, ethtool` |

### Verified fine (no changes needed)

- Debian 13 kernel (cloud and generic): `CONFIG_DEBUG_INFO_BTF=y` for CO-RE, BPF/JIT/XDP/TC-BPF, `sch_cake=m`, veth/bridge/VLAN/virtio-net.
- FreeRADIUS 3.2.7 still uses `/etc/freeradius/3.0`, so harness paths are unchanged.
- `liblqos_python.so` is PyO3 `abi3-py310` and loads under Debian's Python 3.13; no removed-stdlib usage in `src/*.py`.
- The Ubuntu systemd hotfix is inert on Debian (`is_supported_os` guard) and does not block the package postinst.
- Host-built binaries run against trixie's glibc 2.41 and OpenSSL 3.5.

## Phases

- [ ] Phase 0: branch and this plan.
- [ ] Phase 1: shared prerequisite helper and `build_rust_debian.sh`.
- [ ] Phase 2: `build_dpkg.sh` dependency fixes and `build_pkg_debian.sh`.
- [ ] Phase 3: harness Debian guest support (`lab.env`, `lab`, `user-data.yaml`, README).
- [ ] Phase 4: static validation (`bash -n` on touched scripts, harness smoke checks).
- [ ] Phase 5: VM validation: `lab init` / `up` / `configure` / `test` with `LAB_GUEST_OS=debian`.
- [ ] Phase 6 (stretch): install a `build_pkg_debian.sh` artifact inside the Debian guest and re-run the lifecycle test.
- [ ] Phase 7: operator docs and review agents (heckler, reaper, thomas, beck, jonas).

## Validation checklist

- `bash -n` passes for every touched script.
- `LQOS_DISTRO` detection returns `ubuntu` on this host and `debian` under the override.
- The harness image pin verifies against Debian's `SHA512SUMS`.
- The Debian guest reaches the same three RADIUS lifecycle assertions as Ubuntu.
- `lqosd` starts on the Debian guest and pins maps under `/sys/fs/bpf`.
- The default Ubuntu harness path is unchanged (`LAB_GUEST_OS=ubuntu`).

## Risks and open questions

- Host `osinfo-db` may not know `debian13`; the `LAB_OS_VARIANT` override covers that.
- The RouterOS console password step is interactive; the VM run needs an operator at `lab console` once.
- Whether to declare Debian 13 supported in operator-facing docs is a product decision. This branch proves the technical path first.
