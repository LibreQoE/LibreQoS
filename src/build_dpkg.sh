#!/bin/bash
set -euo pipefail

####################################################
# Copyright (c) 2022, Herbert Wolverson and LibreQoE
# This is all GPL2.

BUILD_DATE=$(date +%Y%m%d%H%M)
VERSION_OVERRIDE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --nostamp) BUILD_DATE="" ;;
    --version)
      [[ $# -ge 2 ]] || { echo "--version requires a value" >&2; exit 2; }
      VERSION_OVERRIDE="$2"; shift ;;
    *) ;;
  esac
  shift
done

PACKAGE=libreqos
# The anti-skew rule: the release pipeline builds BOTH packages
# in ONE invocation with ONE version stamp, so the dataplane's exact
# `Depends: libreqos (= <version>)` pin can never self-block on a BUILD_DATE
# mismatch. LIBREQOS_VERSION (or --version) carries that shared stamp; the
# default preserves the historical <VERSION_STRING>.<BUILD_DATE> scheme.
# `--nostamp` clears BUILD_DATE; the `:+` form then yields the bare
# `<VERSION_STRING>` rather than a malformed trailing-dot `<VERSION_STRING>.`
# (docs/apt_repo_distribution_card.md §3: a version-less `3.0.7.` deb was once
# uploaded as an artifact and mistaken for a release).
VERSION="${VERSION_OVERRIDE:-${LIBREQOS_VERSION:-$(cat ./VERSION_STRING)${BUILD_DATE:+.$BUILD_DATE}}}"
PKGVERSION="${PACKAGE}_${VERSION}"
DPKG_DIR=dist/$PKGVERSION-1_amd64
APT_DEPENDENCIES="python3-pip, python3-venv, nano, curl, ca-certificates"
DEBIAN_DIR=$DPKG_DIR/DEBIAN
LQOS_DIR=$DPKG_DIR/opt/libreqos/src
LQOS_STATE_DIR=$DPKG_DIR/opt/libreqos/state
ETC_DIR=$DPKG_DIR/etc
ETC_LIBREQOS_DIR=$DPKG_DIR/etc/libreqos
MOTD_DIR=$DPKG_DIR/etc/update-motd.d
LQOS_FILES=(
  csvToNetworkJSON.py
  configMigrator.py
  integrationCommon.py
  integrationPowercode.py
  integrationNetzur.py
  integrationRestHttp.py
  integrationSonar.py
  integrationUtils.py
  integrationSplynx.py
  integrationVISP.py
  integrationWISPGate.py
  LibreQoS.py
  lqos.example
  lqTools.py
  mikrotikFindIPv6.py
  mikrotik_ipv6.example.toml
  network.example.json
  pythonCheck.py
  qoo_profiles.json
  scheduler.py
  ShapedDevices.example.csv
  shaping_skip_report.py
  systemd_hotfix.sh
  install_caddy.sh
  disable_caddy.sh
  manualNetwork.template.csv
  ../requirements.txt
  update_bridge.sh
  libreqos-apt-source.sh
)

LQOS_BIN_FILES=(
  libreqos-scheduler.service.example
  libreqos-first-run.service.example
  rebuild_python_venv.sh
)

RUSTPROGS=(
  lqosd
  lqos_netplan_helper
  lqtop
  xdp_pping
  lqusers
  lqos_setup
  libreqos-power-profile
  uisp_integration
  lqos_overrides
)

####################################################
# The control-plane deb (distribution split, driver;
# docs/distribution_split_card.md §4).  This package ships the CONTROL PLANE
# only.  The DPDK bridge (binary + runtime closure + EAL PMDs + the bridge
# unit/wrapper + the privilege grants) is a SEPARATE, PROPRIETARY artifact
# distributed as the `libreqos-dataplane` package and fetched at install time by
# update_bridge.sh.  The control plane still ships the lqosd/setup units, the shared
# bridge env conffile, and the operator-facing setup/support tooling.
#   BRIDGE_ROOT  engine tree root (default: the monorepo root)
BRIDGE_PKG="${BRIDGE_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}/packaging"
for f in systemd/libreqos-lqosd.service systemd/libreqos-setup.service \
         systemd/libreqos-power.service \
         libexec/libreqos-setup-check libexec/libreqos-support-bundle \
         libexec/libreqos-api-legacy-cleanup libexec/libreqos-v3-rollback \
         bridge.env.example; do
  if [ ! -f "$BRIDGE_PKG/$f" ]; then
    echo "Missing packaging input: $BRIDGE_PKG/$f" >&2
    exit 1
  fi
done

####################################################
# Clean any previous dist build
rm -rf dist

####################################################
# Bump the build number

####################################################
# The Debian Packaging Bit

# Create the basic directory structure
mkdir -p "$LQOS_DIR"/bin/static2 "$DEBIAN_DIR" "$ETC_DIR" "$ETC_LIBREQOS_DIR" "$LQOS_DIR"/rust "$LQOS_DIR"/bin/dashboards
mkdir -p "$LQOS_STATE_DIR"/topology "$LQOS_STATE_DIR"/shaping "$LQOS_STATE_DIR"/stats "$LQOS_STATE_DIR"/cache "$LQOS_STATE_DIR"/debug "$LQOS_STATE_DIR"/quarantine

# shellcheck disable=SC2086
mkdir -p $MOTD_DIR

# Create the Debian control file
pushd "$DEBIAN_DIR" > /dev/null || exit
cat <<EOF > control
Package: $PACKAGE
Version: $VERSION
Section: net
Priority: optional
Architecture: amd64
Maintainer: Herbert Wolverson <herberticus@gmail.com>
Depends: $APT_DEPENDENCIES
Pre-Depends: libreqos-preflight
Recommends: libreqos-api (= $VERSION)
Replaces: libreqos-v3-bridge, dpdk
Description: LibreQoS v3 — control plane + scheduler (dataplane/API are separate)
 The LibreQoS v3 control plane: the lqosd daemon, the scheduler integration
 hub, the first-run setup, and the web UI. Installs the AF_XDP BPF objects and
 the control-plane systemd census. The DPDK bridge dataplane is a SEPARATE,
 proprietary artifact (the libreqos-dataplane package); the optional API is the
 separate, version-pinned libreqos-api package (this package installs neither,
 and performs no network fetch at install time).
EOF
# The shared production env is operator-editable: declare it a conffile.
cat <<'EOF' > conffiles
/etc/libreqos/bridge.env
EOF
popd > /dev/null || exit

# Build the Rust programs (before the control file, we need to LDD lqosd).
# Keep package artifacts on the full release profile; build_rust.sh --fast is
# intentionally a local-iteration-only shortcut.
pushd rust > /dev/null || exit
# Build only required binaries and artifacts (exclude lqos_support_tool executable)
cargo build --release \
  -p lqosd \
  -p lqos_netplan_helper \
  -p lqtop \
  -p xdp_pping \
  -p lqusers \
  -p lqos_setup \
  -p lqos_power \
  -p uisp_integration \
  -p lqos_python \
  -p lqos_overrides \
  -p lqos_topology
popd > /dev/null || exit

# The dataplane (bridge_rs + its DPDK closure + the EAL PMDs) is NOT built or
# bundled here — it is the separate dataplane artifact built by
# packaging/build_bridge_artifact.sh and fetched at install time by
# update_bridge.sh (the split; docs/distribution_split_card.md §4).

# Create the post-installation file
pushd "$DEBIAN_DIR" > /dev/null || exit
cat <<'EOF' > postinst
#!/bin/bash
set -euo pipefail

# Automatic hugepages (§2, preflight_integration_door_card): attempt the
# RUNTIME 2M allocation first (the fleet's proven path, no reboot in the
# common case), then WRITE THE BOOT-PERSISTENT sysctl drop-in so a reboot does
# not lose the reservation (lqos-74t.37.21; 1st-lqos-01 had runtime-only
# pages). A kernel-cmdline posture (1G/2M) wins and is left alone.
# Best-effort: never fails the install.
HP_WANT="${LIBREQOS_HUGEPAGES_2M:-3072}"
case "$HP_WANT" in
    ''|*[!0-9]*) echo "libreqos postinst: WARNING: invalid LIBREQOS_HUGEPAGES_2M='$HP_WANT'; using 3072" >&2; HP_WANT=3072 ;;
esac
ensure_hugepages() {
local want="$HP_WANT"
local have
have="$(awk '/^HugePages_Total:/{print $2; exit}' /proc/meminfo 2>/dev/null || echo 0)"
if [ "${have:-0}" -ge "$want" ]; then
    echo "libreqos postinst: ${have} hugepages already reserved"
    return 0
fi
echo "$want" > /proc/sys/vm/nr_hugepages 2>/dev/null || true
have="$(awk '/^HugePages_Total:/{print $2; exit}' /proc/meminfo 2>/dev/null || echo 0)"
if [ "${have:-0}" -ge "$want" ]; then
    echo "libreqos postinst: reserved ${have} 2M hugepages at runtime (no reboot needed)"
else
    echo "libreqos postinst: WARNING: ${have}/${want} hugepages after the runtime attempt; a reboot with default_hugepagesz=2M hugepages=${want} (or 1G pages) may be required (packaging/INSTALL.md)" >&2
fi
}
ensure_hugepages

# Boot persistence: the runtime allocation above is lost on reboot. Write the
# managed sysctl drop-in (systemd-sysctl applies it at boot); the preflight and
# libreqos-setup-check read /etc/sysctl.d. A kernel-cmdline 1G/2M posture is
# authoritative; never shadow it with a 2M vm.nr_hugepages key.
persist_hugepages() {
local want="$HP_WANT"
local file="/etc/sysctl.d/80-libreqos-hugepages.conf"
if grep -qE '(^|[[:space:]])(default_hugepagesz|hugepages=)' /proc/cmdline 2>/dev/null; then
    echo "libreqos postinst: kernel cmdline already declares hugepages; not writing $file"
    return 0
fi
if install -d -m 0755 /etc/sysctl.d 2>/dev/null \
   && printf '%s\n' \
        '# Managed by LibreQoS v3: boot-persistent 2M hugepage reservation.' \
        '# The postinst also attempts the runtime allocation for the first boot.' \
        "vm.nr_hugepages = ${want}" > "$file" 2>/dev/null; then
    chmod 0644 "$file" 2>/dev/null || true
    echo "libreqos postinst: wrote boot-persistent hugepage reservation to $file (vm.nr_hugepages = ${want})"
else
    echo "libreqos postinst: WARNING: could not write $file; add 'vm.nr_hugepages = ${want}' manually (packaging/INSTALL.md)" >&2
fi
}
persist_hugepages

# The v3 SERVICE user is `lqosuser` (system, nologin, own group) — NEVER
# `libreqos`: operators log in as `libreqos` on the ISP fleet, and adopting a
# human login as the service identity would key the segment/socket/sudoers to
# an admin account. A pre-existing `libreqos` account is left untouched; a
# previously-created system `libreqos` (an interim) stays harmless. Idempotent.
ensure_service_user() {
if id -u lqosuser >/dev/null 2>&1; then
    return 0
fi
getent group lqosuser >/dev/null 2>&1 || groupadd --system lqosuser 2>/dev/null || true
useradd --system --gid lqosuser --no-create-home --shell /usr/sbin/nologin lqosuser 2>/dev/null || true
if id -u lqosuser >/dev/null 2>&1; then
    echo "libreqos postinst: created the lqosuser service user"
else
    echo "libreqos postinst: WARNING: could not create the lqosuser service user; the bridge control segment stays root-owned" >&2
fi
}
ensure_service_user

set_libreqos_operator_permissions() {
local runtime_paths=()
[ -e /opt/libreqos/src ] && runtime_paths+=("/opt/libreqos/src")
[ -e /opt/libreqos/state ] && runtime_paths+=("/opt/libreqos/state")
[ ${#runtime_paths[@]} -eq 0 ] && return

if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ] && id "$SUDO_USER" >/dev/null 2>&1; then
    chown -R "$SUDO_USER:$SUDO_USER" "${runtime_paths[@]}"
    chmod -R u+rwX "${runtime_paths[@]}" || true
    echo "Granted $SUDO_USER ownership of /opt/libreqos/src and /opt/libreqos/state for SFTP editing."
else
    echo "Unable to determine the installing operator account automatically."
    echo "If you plan to edit LibreQoS files over SFTP, run: sudo chown -R <username>:<username> /opt/libreqos/src /opt/libreqos/state"
fi
}

# - Setup Python dependencies in a root-owned virtual environment. LibreQoS
#   services still run as root, but Python packages no longer mix with
#   apt-managed system site-packages.
/opt/libreqos/src/bin/rebuild_python_venv.sh

# Ensure folder permissions are correct post-install
set_libreqos_operator_permissions

# lqos-u5b: transition the apt sources v2 (repo.libreqos.com) -> v3
# (packages.libreqos.com), deriving the suite from an existing v3 source (or
# LQOS_REPO_SUITE, default beta for a v2->v3 box). No apt-get update here.
if [ -x /opt/libreqos/src/libreqos-apt-source.sh ]; then
  /opt/libreqos/src/libreqos-apt-source.sh activate || true
fi

# - Setup the services (the canonical unit census; card 8dc53187).
# libreqos-bridge/lqosd/setup ship as /etc/systemd/system files in the package;
# the scheduler + first-run are installed here from the runtime tree. The API
# unit is OWNED by the separate libreqos-api package (D2).
install -m 0644 /opt/libreqos/src/bin/libreqos-scheduler.service.example /etc/systemd/system/libreqos-scheduler.service
install -m 0644 /opt/libreqos/src/bin/libreqos-first-run.service.example /etc/systemd/system/libreqos-first-run.service
/bin/systemctl daemon-reload || true

# Legacy-name sweep: stop + disable + REMOVE every pre-canonical unit name so
# an upgrade-in-place cannot keep a stray service (the shared host May-6
# lqos_scheduler ghost class).
unit_stop_disable_remove_if_present() {
local unit="$1"
if /bin/systemctl cat "${unit}" >/dev/null 2>&1 \
    || [ -e "/etc/systemd/system/${unit}" ] \
    || [ -e "/lib/systemd/system/${unit}" ] \
    || [ -e "/usr/lib/systemd/system/${unit}" ]; then
    /bin/systemctl stop "${unit}" || true
    /bin/systemctl disable "${unit}" || true
    /bin/rm -f "/etc/systemd/system/${unit}" || true
fi
}
for legacy_unit in lqosd.service lqos_scheduler.service lqos_setup.service \
                   lqos_node_manager.service lqos_netplan_helper.service \
                   lqos_api.service libreqos-uisp.service libreqos-uisp.timer; do
  unit_stop_disable_remove_if_present "$legacy_unit"
done

# The dataplane (the DPDK bridge) is NOT part of this package (the split,
# docs/distribution_split_card.md §4): fetch + install the portal artifact
# before any service starts. update_bridge.sh is graceful on an unreachable
# endpoint (prints FETCHED / NOT FETCHED, exit 0); the bridge unit is enabled
# ONLY on a successful fetch (fail-closed: a failed fetch leaves the control
# plane running and the bridge inactive).
if [ -x /opt/libreqos/src/update_bridge.sh ]; then
  /opt/libreqos/src/update_bridge.sh --no-restart || true
fi
# ENABLE only — NEVER disable. The bridge unit carries
# ConditionPathExists=/etc/libreqos/lqos.conf, so an enabled unit on a
# setup-mode box does not start until activation writes a config. The dataplane
# package's postinst owns enablement too; leaving the unit's state alone here is
# what makes the four-deb configure order atomic (a later dataplane configure
# cannot be undone by this package). `systemctl enable` never un-masks a unit,
# so operator intent is preserved.
if [ -x /usr/libexec/libreqos/bridge_rs ] \
    && [ -e /etc/systemd/system/libreqos-bridge.service ]; then
  /bin/systemctl enable libreqos-bridge.service 2>/dev/null || true
else
  echo "libreqos postinst: the dataplane is not installed (fetch failed or offline); the bridge unit is left as-is" >&2
fi

# The API is a separate, versioned package now (libreqos-api; D1/D2): this
# package ships NO update_api.sh, NO unit example, and performs NO network
# fetch. Clean up the unowned leftovers of the old floating api3.zip fetch so
# an upgrade cannot keep a stale API running against the new control plane.
# The guard keys on the PACKAGED unit's Environment line: the old example had
# none, while the new libreqos-api unit carries
# `Environment=LQOS_CONFIG=/etc/libreqos/lqos.conf`. dpkg unpacks all packages
# before configuring, so when libreqos-api is part of the same transaction its
# new unit is already on disk and must NOT be touched here — in EITHER
# configure order. The helper is standalone so the four shapes (fresh,
# upgrade-from-fetch, Recommends-skipped, both configure orders) are unit
# tested (scripts/test/test_api_legacy_cleanup.sh).
if [ -x /usr/libexec/libreqos/libreqos-api-legacy-cleanup ]; then
  /usr/libexec/libreqos/libreqos-api-legacy-cleanup || true
fi

# Beta5-ghost purge c2): the retired split package can stay
# registered (Replaces without Conflicts leaves `libreqos-v3-bridge` ii). Purge
# it so an upgrade ends with exactly ONE libreqos package. beta5's postrm may
# STOP the five units — the postinst-action handoff below then re-activates the
# runtime, so the ordering is deterministic. Idempotent.
if dpkg-query -W -f='${Status}' libreqos-v3-bridge 2>/dev/null | grep -q 'install ok installed'; then
  echo "libreqos postinst: purging the retired libreqos-v3-bridge package (one-package posture)"
  dpkg --purge libreqos-v3-bridge >/dev/null 2>&1 || true
  # beta5's postrm stops the five units; re-assert the DATA PLANE only if the
  # dataplane artifact installed it (the postinst-action handoff below
  # re-activates the control plane). Idempotent.
  if [ -x /usr/libexec/libreqos/bridge_rs ]; then
    /bin/systemctl enable libreqos-bridge.service 2>/dev/null || true
    /bin/systemctl restart --no-block libreqos-bridge.service 2>/dev/null || true
  fi
fi

case "$(/opt/libreqos/src/bin/lqos_setup postinst-action)" in
activate_runtime)
  /opt/libreqos/src/bin/lqos_setup activate-runtime
  ;;
activate_setup)
  /opt/libreqos/src/bin/lqos_setup activate-setup
  ;;
*)
  echo "Unknown LibreQoS post-install action." >&2
  exit 1
  ;;
esac
EOF

# Uninstall Script
cat <<EOF > postrm
#!/bin/bash
set +e
/bin/systemctl stop libreqos-bridge libreqos-lqosd libreqos-scheduler libreqos-setup libreqos-power || true
/bin/systemctl disable libreqos-bridge libreqos-lqosd libreqos-scheduler libreqos-setup libreqos-power || true
# libreqos-api is NOT touched here: it belongs to the separate libreqos-api
# package (its own prerm stops/disables it). systemd's Requires=libreqos-lqosd
# stops the API when the control plane is stopped.
/bin/rm -f /etc/systemd/system/lqos_netplan_helper.service || true
# A3: a package REMOVAL restores the host power posture (else uninstalling
# leaves the host pinned at the profile). NOT on upgrade.
if [ "\$1" = "remove" ] || [ "\$1" = "purge" ]; then
  /usr/libexec/libreqos/libreqos-power-profile restore || true
fi
# lqos-74t.37.21: the postinst writes a managed hugepage drop-in. An upgrade
# leaves it (the reservation is still wanted); a purge removes it so a v2
# rollback can provision its own posture.
if [ "\$1" = "purge" ]; then
  rm -f /etc/sysctl.d/80-libreqos-hugepages.conf || true
fi
# lqos-u5b: purge removes ONLY the v3 source file this package created (the
# marker). The shipped v3 keyring is kept while any v3 source remains; the v2
# keyring is never removed.
if [ "\$1" = "purge" ] && [ -f /var/lib/libreqos/apt-source-v3.owned ]; then
  while IFS= read -r src; do
    case "\$src" in
      /etc/apt/sources.list.d/libreqos.list|/etc/apt/sources.list.d/libreqos.sources)
        rm -f "\$src" ;;
    esac
  done < /var/lib/libreqos/apt-source-v3.owned
  rm -f /var/lib/libreqos/apt-source-v3.owned
  # Keep the shipped v3 keyring while ANY v3 source remains (an operator-created
  # .list/.sources or a .v3-disabled backup is still signed-by it). Remove it
  # only when no v3 source is left. The v2 keyring is never touched.
  remaining_v3=0
  for src in /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources \
             /etc/apt/sources.list.d/*.v3-disabled; do
    [ -r "\$src" ] || continue
    if grep -q 'packages\.libreqos\.com' "\$src" 2>/dev/null; then
      remaining_v3=1
      break
    fi
  done
  if [ "\$remaining_v3" -eq 0 ]; then
    rm -f /usr/share/keyrings/libreqos-packages-keyring.gpg
  fi
fi
/bin/systemctl daemon-reload || true
exit 0
EOF
chmod a+x postinst postrm

# Pre-install gate (docs/v2_to_v3_preflight_card.md §1). The checker ships in
# the Pre-Depends 'libreqos-preflight' package, so dpkg guarantees it is
# unpacked+configured before this script runs (a plain Depends would not order
# pre-unpack). A BLOCK (the checker's exit 2) refuses the install unless
# LQOS_PREFLIGHT_FORCE=1 (logged loudly); a WARN (exit 1) proceeds.
cat <<'EOF' > preinst
#!/bin/bash
set -euo pipefail

# Upgrade hygiene (docs/zero_manual_ports_installer_card.md §D):
# (1) STOP the old daemons FIRST — a preinst query against a wedged old lqosd
#     once deadlocked dpkg ~7 min in iU (the retro #5). An interrupted
#     configure must self-heal, so this is best-effort.
for svc in libreqos-lqosd lqosd; do
    /bin/systemctl stop "$svc" >/dev/null 2>&1 || true
done
# (2) REMOVE the superseded unit DROP-INS by KNOWN name: the beta afxdp.conf
#     ExecStart override silently bypassed the wrapper so our EAL args never
#     ran. Only the known-superseded name is removed — operator drop-ins with
#     other names survive.
for stale in /etc/systemd/system/libreqos-bridge.service.d/afxdp.conf \
             /etc/systemd/system/libreqos-bridge.service.d/50-afxdp.conf; do
    if [ -e "$stale" ]; then
        /bin/rm -f "$stale" && echo "libreqos preinst: removed superseded drop-in $stale" >&2
    fi
done
/bin/systemctl daemon-reload >/dev/null 2>&1 || true

CHECKER="${LQOS_PREFLIGHT_CHECKER:-/usr/libexec/libreqos/libreqos-v3-preflight}"
if [ -x "$CHECKER" ]; then
    # `lqos-74t.147`: the preinst is the PRE-INSTALL surface — expected fresh
    # facts are NOTE lines and must not flip PASS to WARN.
    ARGS=(--pre-install)
    if [ -n "${LQOS_PREFLIGHT_PAIR_DIR:-}" ]; then
        ARGS+=(--pair-dir "$LQOS_PREFLIGHT_PAIR_DIR")
    fi
    if timeout 30 "$CHECKER" "${ARGS[@]}"; then
        rc=0
    else
        rc=$?
    fi
    # (3) AUTHORITY + FAIL-CLOSED (E2/D-2): the preflight checker GATES
    #     correctness facts (the NIC pair, the OS/platform, the hugepage/pool
    #     prerequisites), so a TIMEOUT is a FAILURE, not a pass — the doctrine
    #     is fail-closed. The stop-first step above removes the historical
    #     deadlock that a timeout would otherwise mask; LQOS_PREFLIGHT_FORCE=1
    #     remains the explicit operator override (logged).
    if [ "$rc" -eq 124 ]; then
        echo "libreqos preinst: the preflight checker TIMED OUT (30s) — treating as BLOCK (fail-closed); set LQOS_PREFLIGHT_FORCE=1 to override" >&2
        rc=2
    fi
    if [ "$rc" -eq 2 ]; then
        echo "libreqos preinst: the preflight BLOCKed this upgrade" >&2
        if [ "${LQOS_PREFLIGHT_FORCE:-0}" = "1" ]; then
            echo "libreqos preinst: LQOS_PREFLIGHT_FORCE=1 — proceeding anyway (logged)" >&2
        else
            exit 1
        fi
    fi
fi
EOF
chmod a+x preinst
popd > /dev/null || exit

# Copy files into the LibreQoS directory
for file in "${LQOS_FILES[@]}"; do
  if [ ! -f "$file" ]; then
    echo "Missing packaged file: $file" >&2
    exit 1
  fi
done

for file in "${LQOS_FILES[@]}"; do
  cp "$file" "$LQOS_DIR"
done

if [ -f deb-requirements-constraints.txt ]; then
  cp deb-requirements-constraints.txt "$LQOS_DIR"
fi

cp mikrotik_ipv6.example.toml "$ETC_LIBREQOS_DIR"/mikrotik_ipv6.example.toml

# lqos-u5b: ship the v3 public keyring so a fresh v2->v3 install can create the
# packages.libreqos.com source without a manual key fetch.
install -D -m 0644 libreqos-packages-keyring.gpg \
    "$DPKG_DIR/usr/share/keyrings/libreqos-packages-keyring.gpg"

# Ensure helper scripts are executable in the package
for helper_script in update_bridge.sh install_caddy.sh disable_caddy.sh libreqos-apt-source.sh; do
  if [ -f "$LQOS_DIR/$helper_script" ]; then
    chmod a+x "$LQOS_DIR/$helper_script"
  fi
done

# Copy files into the LibreQoS/bin directory
for file in "${LQOS_BIN_FILES[@]}"; do
  cp "bin/$file" "$LQOS_DIR/bin"
done

chmod a+x "$LQOS_DIR/bin/rebuild_python_venv.sh"

# Copy the remove pinned maps
cp rust/remove_pinned_maps.sh "$LQOS_DIR"/rust

####################################################

# Compile the website
pushd rust/lqosd > /dev/null || exit
./copy_files.sh
popd || exit

# Copy newly built Rust files
# - The Python integration Library
CARGO_OUT="${CARGO_TARGET_DIR:-rust/target}"
cp "${CARGO_OUT}/release/liblqos_python.so" "$LQOS_DIR"
# - The main executables
for prog in "${RUSTPROGS[@]}"; do
  cp "${CARGO_OUT}/release/$prog" "$LQOS_DIR"/bin
done

cp -r bin/static2/* "$LQOS_DIR"/bin/static2

####################################################
# Control-plane payload (the split; docs/distribution_split_card.md §4).  The
# dataplane's paths (/usr/libexec/libreqos/{bridge_rs,lib/,pmds/}, the bridge
# unit/wrapper + the sudoers/logind grants) are installed by update_bridge.sh
# from the portal artifact, NOT here.

# The control-plane binary + the operator-facing setup/support tooling (the
# tooling is control-plane: it must run, and report the dataplane's absence,
# without it).
install -D -m 0755 "$LQOS_DIR/bin/lqosd"                          "$DPKG_DIR/usr/libexec/libreqos/lqosd"
install -D -m 0755 "$BRIDGE_PKG/libexec/libreqos-setup-check"    "$DPKG_DIR/usr/libexec/libreqos/libreqos-setup-check"
install -D -m 0755 "$BRIDGE_PKG/libexec/libreqos-support-bundle" "$DPKG_DIR/usr/libexec/libreqos/libreqos-support-bundle"
install -D -m 0755 "$BRIDGE_PKG/libexec/libreqos-api-legacy-cleanup" "$DPKG_DIR/usr/libexec/libreqos/libreqos-api-legacy-cleanup"
# lqos-74t.37.20: the rollback helper was referenced by the docs but never
# shipped. Install it with the control plane (the helper drives the v3 package
# purge + v2 reinstall) and expose the documented PATH-stable command.
install -D -m 0755 "$BRIDGE_PKG/libexec/libreqos-v3-rollback" "$DPKG_DIR/usr/libexec/libreqos/libreqos-v3-rollback"
# The lqosd/setup units (the bridge unit ships in the dataplane artifact).
install -D -m 0644 "$BRIDGE_PKG/systemd/libreqos-lqosd.service"  "$DPKG_DIR/etc/systemd/system/libreqos-lqosd.service"
install -D -m 0644 "$BRIDGE_PKG/systemd/libreqos-setup.service"  "$DPKG_DIR/etc/systemd/system/libreqos-setup.service"
# The host CPU power posture CLI + its boot-persistence unit
# (docs/power_profile_work_order.md). The CLI is exposed on PATH via /usr/bin.
install -D -m 0755 "$LQOS_DIR/bin/libreqos-power-profile"        "$DPKG_DIR/usr/libexec/libreqos/libreqos-power-profile"
install -D -m 0644 "$BRIDGE_PKG/systemd/libreqos-power.service"  "$DPKG_DIR/etc/systemd/system/libreqos-power.service"
mkdir -p "$DPKG_DIR/usr/bin"
ln -sf /usr/libexec/libreqos/libreqos-power-profile "$DPKG_DIR/usr/bin/libreqos-power-profile"
# Path-stable operator commands (lqos-74t.37.20): the docs invoke the rollback
# helper and lqos_setup by bare name. /usr/sbin is root's operator PATH; the
# absolute /usr/libexec (helper) and /opt (setup binary) paths remain shipped.
mkdir -p "$DPKG_DIR/usr/sbin"
ln -sf /usr/libexec/libreqos/libreqos-v3-rollback "$DPKG_DIR/usr/sbin/libreqos-v3-rollback"
ln -sf /opt/libreqos/src/bin/lqos_setup "$DPKG_DIR/usr/sbin/lqos_setup"
# The shared production env (a conffile).
install -D -m 0644 "$BRIDGE_PKG/bridge.env.example"              "$DPKG_DIR/etc/libreqos/bridge.env"
# The DPDK runtime closure, the ld.so.conf.d entry, and the EAL PMDs (with the
# F1 scan-path rewrite) are built INTO the dataplane artifact by
# packaging/build_bridge_artifact.sh and installed by update_bridge.sh (the
# split).  Nothing dataplane-side ships in this deb.

cat <<EOF > "$LQOS_DIR/bin/dashboards/default.json"
{"name":"default","entries":[
  {"name":"Shaped/Unshaped Pie","tag":"shapedUnshaped","size":2},
  {"name":"Last 5 Minutes Throughput","tag":"throughputRing","size":4},
  {"name":"Total TCP Retransmits","tag":"totalRetransmits","size":4},
  {"name":"RAM Utilization","tag":"ram","size":2},
  {"name":"Throughput Packets/Second","tag":"throughputPps","size":2},
  {"name":"Round-Trip Time Histogram","tag":"rttHistogram","size":2},
  {"name":"Total Cake Stats","tag":"totalCakeStats","size":4},
  {"name":"Tracked Flows Counter","tag":"trackedFlowsCount","size":2},
  {"name":"CPU Utilization","tag":"cpu","size":2},
  {"name":"Network Tree Sankey","tag":"networkTreeSankey","size":6},
  {"name":"Network Tree Summary","tag":"treeSummary","size":6},
  {"name":"Top 10 Downloaders (Visual)","tag":"top10downloadersV","size":6},
  {"name":"Top 10 Downloaders","tag":"top10downloaders","size":6},
  {"name":"Worst 10 Round-Trip Time (Visual)","tag":"worst10downloadersV","size":6},
  {"name":"Worst 10 Round-Trip Time","tag":"worst10downloaders","size":6},
  {"name":"Worst 10 Retransmits (Visual)","tag":"worst10retransmitsV","size":6},
  {"name":"Worst 10 Retransmits","tag":"worst10retransmits","size":6},
  {"name":"Top 10 Flows (total bytes)","tag":"top10flowsBytes","size":6},
  {"name":"Top 10 Flows (rate)","tag":"top10flowsRate","size":6},
  {"name":"Top 10 Endpoints by Country","tag":"top10endpointsCountry","size":6},
  {"name":"Ether Protocols","tag":"etherProtocols","size":6},
  {"name":"IP Protocols","tag":"ipProtocols","size":6},
  {"name":"Combined Top 10 Box","tag":"combinedTop10","size":6},
  {"name":"Circuits At Capacity","tag":"circuitCapacity","size":6},
  {"name":"Tree Nodes At Capacity","tag":"treeCapacity","size":6}
]}
EOF

####################################################
# Add Message of the Day
pushd "$MOTD_DIR" > /dev/null || exit
cat <<EOF > 99-libreqos
#!/bin/bash
MY_IP=$(hostname -I | cut -d' ' -f1)
echo \"\"
echo "LibreQoS Traffic Shaper is installed on this machine."
echo "Point a browser at http://\$MY_IP:9123/ to manage it."
echo \"\"
EOF
popd || exit

####################################################
# lqos_api is a separate, versioned package (libreqos-api; D1/D2): this
# package ships neither the binary nor the unit nor a fetch script, and its
# postinst performs no network access. The source tree keeps update_api.sh as a
# developer tool only (versioned URL + sha256, never called from a package).
####################################################
# Deterministic file modes (reproducible-build rider, the deb
# must not inherit the BUILDER's umask.  A checkout with umask 002 ships
# 0664/0775 files and produces a false file-list diff against a umask-022 build
# (the 860-line case,. Remove the group/other write bit across the
# packaged tree; restrictive modes such as /etc/sudoers.d/libreqos-bridge
# (0440) carry no group/other write bit and are unaffected.
chmod -R go-w "$DPKG_DIR"

# Assemble the package
dpkg-deb --root-owner-group --build "$DPKG_DIR"
