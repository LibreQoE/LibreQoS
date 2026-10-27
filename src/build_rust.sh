#!/bin/bash
#
# LibreQoS v3 monorepo developer bootstrap (docs/dev_experience_bootstrap_monorepo_card.md).
#
#   ./build_rust.sh [check|build|demo|install|clean] [options]
#
# No subcommand => build (v2 muscle memory). `bootstrap` is an alias for build.
# This is a PRIVATE-repo developer tool: the engine is in-tree at
# rust/libreqos_dpdk. Public projection: the engine tree is absent, `build`
# builds the control plane only and names the private repo.
#
# Run this from a source clone (for example ~/libreqos-private), not from the
# package-managed /opt/libreqos/src tree. `install` writes into the package
# paths; it does not install the clone itself.
#
# It is a developer build/install tool, NOT a package builder. Release
# packaging is a separate pipeline and is not invoked here.
set -euo pipefail

SELF_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SRC_ROOT="$SELF_DIR"                       # <repo>/src/src
REPO_ROOT="$(cd "$SRC_ROOT/../.." && pwd)" # <repo>
CP_WS="$SRC_ROOT/rust"
BIN_DIR="$SRC_ROOT/bin"
BACKUP_ROOT="/var/lib/libreqos/dev-install-backup"
# ALL LQOS_DEV_* seams require this one explicit switch. Production installs
# ignore them and use only the absolute package layout / system sudo/systemctl.
DEV_TEST_MODE=0
if [ "${LQOS_DEV_TEST_MODE:-0}" = "1" ]; then
  DEV_TEST_MODE=1
  ENGINE_WS="${LQOS_DEV_ENGINE_WS:-$REPO_ROOT/rust/libreqos_dpdk}"
  BUILD_DIR="${LQOS_DEV_BUILD_DIR:-$REPO_ROOT/build/dev}"
  DEST_ROOT="${LQOS_DEV_DEST_ROOT:-}"
  SUDO="${LQOS_DEV_SUDO:-sudo}"
  [ -n "${LQOS_DEV_NO_SUDO:-}" ] && SUDO=""
  SYSTEMCTL="${LQOS_DEV_SYSTEMCTL:-systemctl}"
  ARTIFACT_ROOT="${LQOS_DEV_ARTIFACT_ROOT:-}"
  STATIC2_SOURCE="${LQOS_DEV_STATIC2_SOURCE:-$BIN_DIR/static2}"
  LIB_SOURCE="${LQOS_DEV_LIB_SOURCE:-$SRC_ROOT/liblqos_python.so}"
else
  ENGINE_WS="$REPO_ROOT/rust/libreqos_dpdk"
  BUILD_DIR="$REPO_ROOT/build/dev"
  DEST_ROOT=""
  SUDO="sudo"
  SYSTEMCTL="systemctl"
  ARTIFACT_ROOT=""
  STATIC2_SOURCE="$BIN_DIR/static2"
  LIB_SOURCE="$SRC_ROOT/liblqos_python.so"
fi
ENGINE_CRATE="$ENGINE_WS/bridge_rs"
LOCAL_VENV="$BUILD_DIR/venv"
rooted() { printf '%s%s' "$DEST_ROOT" "$1"; }

# The release DPDK installs into the private prefix, not the default pkg-config
# search path. Honor it when present (the 24.04 build host).
if [ -d /usr/libexec/libreqos/pkgconfig ]; then
  export PKG_CONFIG_PATH="/usr/libexec/libreqos/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  export LD_LIBRARY_PATH="/usr/libexec/libreqos${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

CP_PACKAGES=(
  lqosd lqos_netplan_helper lqtop xdp_pping lqusers lqos_setup
  lqos_power uisp_integration lqos_overrides lqos_topology lqos_python
)
CP_BINARIES=(
  lqosd lqos_netplan_helper lqtop xdp_pping lqusers lqos_setup
  libreqos-power-profile uisp_integration lqos_overrides
)
# Install payload mirrors the packaged control-plane file set. Keep this list
# complete; the behavior suite asserts the installed payload.
CP_SOURCE_FILES=(
  csvToNetworkJSON.py configMigrator.py integrationCommon.py
  integrationPowercode.py integrationNetzur.py integrationRestHttp.py
  integrationSonar.py integrationUtils.py integrationSplynx.py
  integrationVISP.py integrationWISPGate.py LibreQoS.py lqos.example
  lqTools.py mikrotikFindIPv6.py mikrotik_ipv6.example.toml
  network.example.json pythonCheck.py qoo_profiles.json scheduler.py
  ShapedDevices.example.csv shaping_skip_report.py systemd_hotfix.sh
  install_caddy.sh disable_caddy.sh manualNetwork.template.csv
  ../requirements.txt update_api.sh update_bridge.sh libreqos-apt-source.sh
)
# Install payload mirrors the packaged control-plane bin file set.
CP_BIN_SOURCE_FILES=(
  libreqos-scheduler.service.example libreqos-first-run.service.example
  rebuild_python_venv.sh
)

FAST=0
NO_VENV=0
WITH_DATAPLANE=0
SKIP_DATAPLANE=0
NO_RESTART=0
DRY_RUN=0
NO_BUILD=0
SKIP_ENGINE=0
VERIFY_KEY="${LIBREQOS_LICENSE_VERIFY_KEY:-}"
VERIFY_KEY_FILE="${LIBREQOS_LICENSE_VERIFY_KEY_FILE:-}"

usage() {
  cat <<EOF
Usage: $0 [check|build|demo|install|clean] [options]

Commands:
  check          validate rustup/cargo, engine tree, DPDK 25.11, glibc 2.39
  build          build the control plane + in-tree bridge_rs; static2; local venv
  demo           run the local AF_PACKET demo (root; re-execs under sudo)
  install        install the dev build on a package-installed box (Herbert mode)
  clean          remove target/, build/dev/, the local venv, generated static2

Options:
  --fast                  local fast-release profile
  --no-venv               skip the local Python venv creation
  --with-dataplane        install/restart the built bridge (verify-key warning only)
  --skip-dataplane        control-plane only (default)
  --no-restart            do not restart the runtime units
  --no-build              install uses the existing local build
  --verify-key HEX        issuer ed25519 verify key embedded in the bridge
  --verify-key-file PATH  read the key from PATH
  --dry-run               print the build/install/demo plan; change nothing
  -h, --help              this help
EOF
}

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

cmd="build"
if [ $# -gt 0 ]; then
  case "$1" in
    check|build|demo|install|clean) cmd="$1"; shift ;;
    bootstrap) cmd="build"; shift ;;
    -h|--help) usage; exit 0 ;;
    --*) : ;;
    *) die "unknown command '$1' (check|build|demo|install|clean)" ;;
  esac
fi

# `demo` is a pass-through: it owns its own --dry-run/selftest options.
if [ "$cmd" = "demo" ]; then
  exec python3 "$SRC_ROOT/dev_experience/demo.py" "$@"
fi

while [ $# -gt 0 ]; do
  case "$1" in
    --fast) FAST=1 ;;
    --no-venv) NO_VENV=1 ;;
    --with-dataplane) WITH_DATAPLANE=1 ;;
    --skip-dataplane) SKIP_DATAPLANE=1 ;;
    --no-restart) NO_RESTART=1 ;;
    --no-build) NO_BUILD=1 ;;
    --dry-run) DRY_RUN=1 ;;
    --verify-key) VERIFY_KEY="${2:?--verify-key needs a value}"; shift ;;
    --verify-key=*) VERIFY_KEY="${1#*=}" ;;
    --verify-key-file) VERIFY_KEY_FILE="${2:?--verify-key-file needs a path}"; shift ;;
    --verify-key-file=*) VERIFY_KEY_FILE="${1#*=}" ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

if [ "$WITH_DATAPLANE" -eq 1 ] && [ "$SKIP_DATAPLANE" -eq 1 ]; then
  die "--with-dataplane and --skip-dataplane are mutually exclusive"
fi

glibc_version() {
  # Do not use `head` here: under `set -o pipefail`, closing the pipe early
  # makes ldd get SIGPIPE (141) on hosts that print a multi-line version.
  ldd --version 2>/dev/null | sed -n '1s/.* \([0-9][0-9]*\.[0-9][0-9]*\).*/\1/p'
}

engine_present() { [ -f "$ENGINE_WS/Cargo.toml" ] && [ -f "$ENGINE_CRATE/Cargo.toml" ]; }

resolve_key() {
  # Precedence: --verify-key/--verify-key-file, env, packaging/.dev-key.
  if [ -n "$VERIFY_KEY" ]; then KEY_SOURCE=env; return; fi
  if [ -n "$VERIFY_KEY_FILE" ]; then
    [ -r "$VERIFY_KEY_FILE" ] || die "verify key file not readable: $VERIFY_KEY_FILE"
    VERIFY_KEY="$(tr -d '[:space:]' < "$VERIFY_KEY_FILE")"
    KEY_SOURCE=file; return
  fi
  if [ -n "${LIBREQOS_LICENSE_VERIFY_KEY_FILE:-}" ]; then
    [ -r "$LIBREQOS_LICENSE_VERIFY_KEY_FILE" ] || die "verify key file not readable: $LIBREQOS_LICENSE_VERIFY_KEY_FILE"
    VERIFY_KEY="$(tr -d '[:space:]' < "$LIBREQOS_LICENSE_VERIFY_KEY_FILE")"
    KEY_SOURCE=file; return
  fi
  if [ -f "$REPO_ROOT/packaging/.dev-key" ]; then
    VERIFY_KEY="$(tr -d '[:space:]' < "$REPO_ROOT/packaging/.dev-key")"
    KEY_SOURCE=dev-key; return
  fi
  KEY_SOURCE=placeholder
}

expected_fingerprint() {
  # Must match license.rs: SHA-256 of the raw 32-byte ed25519 PUBLIC key,
  # first 4 bytes as lowercase hex (8 chars).
  printf '%s' "$VERIFY_KEY" | tr -d '[:space:]:-' | python3 -c '
import sys, hashlib, binascii
raw = sys.stdin.read().strip()
try:
    print(hashlib.sha256(binascii.unhexlify(raw)).hexdigest()[:8])
except Exception:
    print("")
'
}

license_probe_accept() { # probe_output expected_fingerprint
  local out="$1" want="$2"
  [ -n "$want" ] || return 1
  printf '%s\n' "$out" | grep -qx "embedded_is_placeholder=false fingerprint=${want}"
}

is_placeholder_key() {
  case "${1:-}" in
    ""|0000000000000000000000000000000000000000000000000000000000000000) return 0 ;;
  esac
  return 1
}

check_host() {
  local mode="$1" dpdk glibc
  command -v rustup >/dev/null 2>&1 || die "rustup is not installed. Install from https://rustup.rs (do not use root)."
  command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH (source \$HOME/.cargo/env)."
  rustup -V >/dev/null 2>&1 || die "rustup is present but not usable."
  [ -d "$CP_WS" ] || die "control-plane workspace missing: $CP_WS"
  if ! engine_present; then
    if [ "$mode" = "install" ] && [ "$WITH_DATAPLANE" -eq 1 ]; then
      die "the in-tree engine is absent ($ENGINE_CRATE); clone LibreQoS/libreqos-private for --with-dataplane."
    fi
    log "NOTE: the in-tree engine is absent; this is the public projection — control plane only."
    if [ "$mode" != "install" ]; then
      log "NOTE: DPDK/dataplane prerequisites skipped for the public control-plane build."
      return 0
    fi
  fi
  if [ "$mode" = "install" ] && [ "$WITH_DATAPLANE" -eq 0 ]; then
    log "NOTE: control-plane-only install; DPDK/engine prerequisites are not required."
    return 0
  fi
  dpdk="$(pkg-config --modversion libdpdk 2>/dev/null || true)"
  if [ "$dpdk" != "25.11.0" ]; then
    cat >&2 <<EOF
ERROR: DPDK 25.11 is required to build the v3 dataplane (found '${dpdk:-none}').
This script never installs/substitutes silently. Build/install the pinned DPDK:
  curl -fLO https://fast.dpdk.org/rel/dpdk-25.11.tar.xz
  tar -xf dpdk-25.11.tar.xz && cd dpdk-25.11
  meson setup build --prefix=/usr --libdir=libexec/libreqos --buildtype=release \\
    -Ddrivers_install_subdir=pmds -Dexamples= -Dtests=false -Denable_docs=false
  ninja -C build && sudo ninja -C build install
  export PKG_CONFIG_PATH=/usr/libexec/libreqos/pkgconfig
EOF
    exit 1
  fi
  glibc="$(glibc_version)"
  if [ "$glibc" != "2.39" ]; then
    cat >&2 <<EOF
ERROR: the release build target is Ubuntu 24.04 / glibc 2.39 (found '${glibc:-unknown}').
Do not build a 24.04-target release artifact on 26.04/glibc 2.41. Use a 24.04 host/container with
DPDK 25.11, or use the prebuilt/local artifacts on this host and follow the dev
instructions (install --no-build).
EOF
    exit 1
  fi
}

build_local() {
  local profile target_key key_args=()
  if [ "$FAST" -eq 1 ]; then profile="fast-release"; target_key="fast-release"; else profile="release"; target_key="release"; fi
  resolve_key
  if [ "$KEY_SOURCE" = "env" ] || [ "$KEY_SOURCE" = "file" ]; then
    key_args=(-e "LIBREQOS_LICENSE_VERIFY_KEY=$VERIFY_KEY")
  elif [ "$KEY_SOURCE" = "dev-key" ]; then
    key_args=(-e "LIBREQOS_LICENSE_VERIFY_KEY=$VERIFY_KEY")
    log "WARNING: using packaging/.dev-key; this is a DEV verify key, not a license. Production grants will not verify; cap 1000."
  else
    log "WARNING: no verify key; building with the all-zero placeholder. This needs no license/key and is valid for development; production grants will not verify; cap 1000."
  fi

  log "==> building the control plane ($profile)"
  local -a pkg_args=()
  for pkg in "${CP_PACKAGES[@]}"; do pkg_args+=(-p "$pkg"); done
  ( cd "$CP_WS" && cargo build --profile "$profile" "${pkg_args[@]}" )
  mkdir -p "$BIN_DIR"
  for prog in "${CP_BINARIES[@]}"; do
    local src="$CP_WS/target/$target_key/$prog"
    [ -f "$src" ] || die "expected control-plane binary missing: $src"
    cp "$src" "$BIN_DIR/$prog.new"
    mv "$BIN_DIR/$prog.new" "$BIN_DIR/$prog"
  done
  cp "$CP_WS/target/$target_key/liblqos_python.so" "$SRC_ROOT/liblqos_python.so.new"
  mv "$SRC_ROOT/liblqos_python.so.new" "$SRC_ROOT/liblqos_python.so"

  log "==> regenerating static2 (copy_files/esbuild)"
  ( cd "$CP_WS/lqosd" && ./copy_files.sh )

  if [ "$NO_VENV" -eq 0 ]; then
    log "==> creating the local Python venv ($LOCAL_VENV)"
    mkdir -p "$BUILD_DIR"
    python3 -m venv --clear "$LOCAL_VENV"
    "$LOCAL_VENV/bin/python" -m pip install --upgrade pip
    "$LOCAL_VENV/bin/python" -m pip install -r "$REPO_ROOT/src/requirements.txt"
  fi

  if engine_present && [ "$SKIP_ENGINE" -eq 0 ]; then
    local source_rev touched=0 f built_rev
    source_rev="$(git -C "$REPO_ROOT" rev-parse --short=12 HEAD 2>/dev/null || true)"
    if [ -z "$source_rev" ]; then source_rev="${LQOS_BUILD_REV:-unknown}"; fi
    [ -n "$source_rev" ] || die "cannot resolve LQOS_BUILD_REV/source revision for bridge_rs"
    log "==> building the in-tree engine (bridge_rs, source rev $source_rev)"
    mkdir -p "$BUILD_DIR"
    # `LIBREQOS_LICENSE_VERIFY_KEY` is read through option_env!, which build.rs
    # does not track. Touch both crate roots (the release builder's guard) on
    # every dev build so a changed key cannot reuse a cached bridge.
    for f in "$ENGINE_CRATE/src/lib.rs" "$ENGINE_CRATE/src/main.rs"; do
      if [ -f "$f" ]; then touch "$f"; touched=1; fi
    done
    if [ "$touched" -eq 0 ]; then
      ( cd "$ENGINE_WS" && cargo clean -p bridge_rs --release ) >/dev/null 2>&1 || true
    fi
    ( cd "$ENGINE_WS" && env "LQOS_BUILD_REV=$source_rev" "${key_args[@]}" cargo build --release -p bridge_rs )
    built_rev="$(timeout 5 env LD_LIBRARY_PATH="/usr/libexec/libreqos/lib:${LD_LIBRARY_PATH:-}" "$ENGINE_WS/target/release/bridge_rs" --print-build-rev 2>/dev/null || true)"
    [ "$built_rev" = "$source_rev" ] || die "built bridge rev '$built_rev' != current source rev '$source_rev' (stale cache?)"
    printf '%s\n' "$KEY_SOURCE" > "$BUILD_DIR/bridge-key.source"
  else
    log "NOTE: engine absent (public projection); control plane only. Clone LibreQoS/libreqos-private for bridge_rs."
  fi
  log "==> build complete"
}

artifact_path() { # artifact_path PATH
  local name="$1"
  if [ "$FAST" -eq 1 ]; then
    printf '%s' "${ARTIFACT_ROOT}$CP_WS/target/fast-release/$name"
  else
    printf '%s' "${ARTIFACT_ROOT}$CP_WS/target/release/$name"
  fi
}

engine_artifact() { printf '%s' "${ARTIFACT_ROOT}$ENGINE_WS/target/release/bridge_rs"; }

warn_bridge_key_state() {
  # LIBREQOS_LICENSE_VERIFY_KEY is the issuer PUBLIC verification key embedded
  # at build time; the license is a signed grant read at runtime. A bridge
  # built with a dev/placeholder key is still a valid dev bridge. Warn only:
  # production grants will not verify and the cap falls back to 1000.
  local bin="$1" out expected probe_rc=0 source
  local key_source_file="$BUILD_DIR/bridge-key.source"
  source="$KEY_SOURCE"
  if [ -r "$key_source_file" ]; then source="$(cat "$key_source_file")"; fi
  if [ "$source" = "dev-key" ] || [ "$source" = "placeholder" ] || is_placeholder_key "$VERIFY_KEY"; then
    log "WARNING: no/dev verify key embedded (source '$source'); production grants will not verify; cap 1000."
  fi
  # Never execute an unknown artifact with the probe arg: a binary without the
  # print-and-exit probe would fall through to run_bridge. Warn and continue.
  if ! grep -aq -- '--print-license-key' "$bin"; then
    log "WARNING: built bridge has no --print-license-key probe (missing/stale artifact); proceeding without embedded-key verification."
    return 0
  fi
  expected="$(expected_fingerprint)"
  out="$(timeout 5 env LD_LIBRARY_PATH="/usr/libexec/libreqos/lib:${LD_LIBRARY_PATH:-}" "$bin" --print-license-key 2>&1)" || probe_rc=$?
  if [ "$probe_rc" -eq 124 ]; then
    log "WARNING: built bridge license probe timed out; proceeding without embedded-key verification."
  elif [ "$probe_rc" -ne 0 ]; then
    log "WARNING: built bridge license probe exited $probe_rc; proceeding without embedded-key verification."
  elif [ -z "$out" ] || ! printf '%s\n' "$out" | grep -q 'embedded_is_placeholder='; then
    log "WARNING: built bridge license probe produced no probe line; proceeding without embedded-key verification."
  elif ! license_probe_accept "$out" "$expected"; then
    log "WARNING: built bridge verify-key probe does not match the current key; production grants will not verify; cap 1000."
  fi
  return 0
}

backup_path() { # backup_path DEST -> copy into the timestamped backup
  local src="$1" rel="${1#"$DEST_ROOT"}"
  local dst="$BACKUP_DIR/$rel"
  $SUDO mkdir -p "$(dirname "$dst")"
  if [ -e "$src" ] || [ -L "$src" ]; then $SUDO cp -a "$src" "$dst"; fi
}

build_plan() {
  log "DRY-RUN build plan (no target/venv/static2 writes)"
  log "  control plane: $CP_WS (${CP_PACKAGES[*]})"
  log "  engine:        $ENGINE_WS (bridge_rs)"
  log "  static2:       $CP_WS/lqosd/copy_files.sh"
  log "  venv:          $LOCAL_VENV"
  log "  profile:       $([ "$FAST" -eq 1 ] && echo fast-release || echo release)"
}

install_plan() {
  log "DRY-RUN install plan (no backup/copy/restart/venv writes)"
  log "  lqosd:   $(artifact_path lqosd) -> $(rooted /usr/libexec/libreqos/lqosd)"
  log "  tools:   control-plane payload -> $(rooted /opt/libreqos/src/bin)"
  log "  python:  liblqos_python.so -> $(rooted /opt/libreqos/src)"
  log "  static2: $STATIC2_SOURCE -> $(rooted /opt/libreqos/src/bin/static2)"
  log "  power:   $(artifact_path libreqos-power-profile) -> $(rooted /usr/libexec/libreqos/libreqos-power-profile) (+ /usr/bin symlink)"
  if [ "$WITH_DATAPLANE" -eq 1 ]; then
    log "  bridge:  $(engine_artifact) -> $(rooted /usr/libexec/libreqos/bridge_rs)"
  else
    log "  bridge:  untouched (control-plane only; use --with-dataplane)"
  fi
  log "  backup:  $(rooted "$BACKUP_ROOT")/<timestamp>"
  if [ "$NO_RESTART" -eq 0 ]; then
    if [ "$WITH_DATAPLANE" -eq 1 ]; then
      log "  restart: libreqos-lqosd -> libreqos-bridge -> libreqos-scheduler -> libreqos-api"
    else
      log "  restart: libreqos-lqosd -> libreqos-scheduler -> libreqos-api"
    fi
  else
    log "  restart: none (--no-restart)"
  fi
}

install_local() {
  if [ "$SKIP_DATAPLANE" -eq 1 ]; then WITH_DATAPLANE=0; fi
  if [ "$WITH_DATAPLANE" -eq 0 ]; then
    SKIP_ENGINE=1
    log "NOTE: control-plane-only install; the bridge/dataplane is untouched."
  fi
  if [ "$DRY_RUN" -eq 1 ]; then install_plan; return 0; fi
  [ -d "$(rooted /opt/libreqos/src)" ] || die "install requires a package-installed box ($(rooted /opt/libreqos/src) is missing)"
  [ -d "$(rooted /usr/libexec/libreqos)" ] || die "install requires a package-installed box ($(rooted /usr/libexec/libreqos) is missing)"
  if [ "$WITH_DATAPLANE" -eq 1 ] && ! engine_present; then
    die "the in-tree engine is absent; --with-dataplane requires LibreQoS/libreqos-private."
  fi
  if [ "$NO_BUILD" -eq 0 ]; then
    check_host install
    build_local
  else
    log "NOTE: --no-build; using the existing local artifacts (no host/DPDK/glibc check)."
  fi
  resolve_key
  local bridge_bin; bridge_bin="$(engine_artifact)"
  if [ "$WITH_DATAPLANE" -eq 1 ]; then
    [ -f "$bridge_bin" ] || die "bridge_rs not built: $bridge_bin"
    warn_bridge_key_state "$bridge_bin"
  fi

  local stamp; stamp="$(date -u +%Y%m%dT%H%M%S%N)-$$"
  BACKUP_DIR="$(rooted "$BACKUP_ROOT")/$stamp"
  ROLLBACK_LIST="$(mktemp)"
  INSTALL_ACTIVE=1
  trap 'rc=$?; if [ "${INSTALL_ACTIVE:-0}" -eq 1 ]; then rollback_install; fi; exit $rc' EXIT

  install_file() { # install_file SRC DST MODE
    local src="$1" dst="$2" mode="$3"
    [ -e "$src" ] || die "missing build artifact: $src"
    if [ "$DEV_TEST_MODE" -eq 1 ] && [ -n "${LQOS_DEV_FAIL_AT:-}" ] && [ "$(basename "$dst")" = "$LQOS_DEV_FAIL_AT" ]; then
      die "test-injected install failure at $dst"
    fi
    backup_path "$dst"
    printf '%s\n' "$dst" >> "$ROLLBACK_LIST"
    $SUDO mkdir -p "$(dirname "$dst")"
    $SUDO cp -a "$src" "$dst.new"
    $SUDO mv -f "$dst.new" "$dst"
    if [ "$DEV_TEST_MODE" -eq 0 ] || [ -z "${LQOS_DEV_NO_CHOWN:-}" ]; then $SUDO chown root:root "$dst"; fi
    $SUDO chmod "$mode" "$dst"
  }

  install_dir() { # install_dir SRC DST
    local src="$1" dst="$2"
    # NOTE: static2 uses two renames (old aside, then new in). This is deliberately
    # NOT atomic: a reader can briefly observe the old path moved away before the
    # new directory appears. That window is acceptable for a dev install.
    [ -d "$src" ] || die "missing build directory: $src"
    backup_path "$dst"
    printf '%s\n' "$dst" >> "$ROLLBACK_LIST"
    $SUDO mkdir -p "$(dirname "$dst")"
    $SUDO rm -rf "$dst.new"
    $SUDO cp -a "$src" "$dst.new"
    if [ -e "$dst" ]; then $SUDO mv "$dst" "$dst.old.$$"; fi
    $SUDO mv "$dst.new" "$dst"
    $SUDO rm -rf "$dst.old.$$"
    if [ "$DEV_TEST_MODE" -eq 0 ] || [ -z "${LQOS_DEV_NO_CHOWN:-}" ]; then $SUDO chown -R root:root "$dst"; fi
    $SUDO find "$dst" -type d -exec chmod 0755 {} +
    $SUDO find "$dst" -type f -exec chmod 0644 {} +
  }

  local opt_src opt_bin exec_root
  opt_src="$(rooted /opt/libreqos/src)"
  opt_bin="$opt_src/bin"
  exec_root="$(rooted /usr/libexec/libreqos)"

  install_file "$(artifact_path lqosd)" "$exec_root/lqosd" 0755
  for prog in "${CP_BINARIES[@]}"; do
    [ "$prog" = "lqosd" ] && continue
    [ "$prog" = "libreqos-power-profile" ] && continue
    install_file "$(artifact_path "$prog")" "$opt_bin/$prog" 0755
  done
  install_file "$(artifact_path libreqos-power-profile)" "$exec_root/libreqos-power-profile" 0755
  install_file "$LIB_SOURCE" "$opt_src/liblqos_python.so" 0644

  local f src_file dst_file
  for f in "${CP_SOURCE_FILES[@]}"; do
    if [ "$f" = "../requirements.txt" ]; then
      src_file="$REPO_ROOT/src/requirements.txt"
      dst_file="$opt_src/requirements.txt"
    else
      src_file="$SRC_ROOT/$f"
      dst_file="$opt_src/$f"
    fi
    [ -e "$src_file" ] || die "missing control-plane payload: $src_file"
    case "$f" in
      *.sh) install_file "$src_file" "$dst_file" 0755 ;;
      *) install_file "$src_file" "$dst_file" 0644 ;;
    esac
  done
  install_file "$SRC_ROOT/rust/remove_pinned_maps.sh" "$opt_src/rust/remove_pinned_maps.sh" 0755

  for f in "${CP_BIN_SOURCE_FILES[@]}"; do
    [ -e "$SRC_ROOT/bin/$f" ] || die "missing control-plane bin payload: $SRC_ROOT/bin/$f"
    case "$f" in
      *.sh) install_file "$SRC_ROOT/bin/$f" "$opt_bin/$f" 0755 ;;
      *) install_file "$SRC_ROOT/bin/$f" "$opt_bin/$f" 0644 ;;
    esac
  done

  install_dir "$STATIC2_SOURCE" "$opt_bin/static2"
  if [ "$WITH_DATAPLANE" -eq 1 ]; then install_file "$bridge_bin" "$exec_root/bridge_rs" 0755; fi

  # Power CLI on PATH (the packaged /usr/bin symlink).
  local power_link; power_link="$(rooted /usr/bin/libreqos-power-profile)"
  backup_path "$power_link"
  printf '%s\n' "$power_link" >> "$ROLLBACK_LIST"
  $SUDO mkdir -p "$(dirname "$power_link")"
  $SUDO ln -sfn "$(rooted /usr/libexec/libreqos/libreqos-power-profile)" "$power_link"
  if [ "$DEV_TEST_MODE" -eq 0 ] || [ -z "${LQOS_DEV_NO_CHOWN:-}" ]; then $SUDO chown -h root:root "$power_link"; fi

  if [ "$NO_RESTART" -eq 0 ]; then
    # The installed venv is rebuilt only when services may be restarted. Back
    # it up before the rebuild so a failed rebuild is covered by rollback too.
    if [ "$DEV_TEST_MODE" -eq 1 ] && [ -n "${LQOS_DEV_SKIP_VENV:-}" ]; then
      log "NOTE: test seam: venv rebuild skipped."
    else
      local venv_dir; venv_dir="$(rooted /opt/libreqos/venv)"
      backup_path "$venv_dir"
      printf '%s\n' "$venv_dir" >> "$ROLLBACK_LIST"
      local rebuild_cmd="$opt_bin/rebuild_python_venv.sh"
      if [ "$DEV_TEST_MODE" -eq 1 ] && [ -n "${LQOS_DEV_REBUILD_VENV:-}" ]; then
        rebuild_cmd="$LQOS_DEV_REBUILD_VENV"
      fi
      if [ -x "$rebuild_cmd" ]; then
        $SUDO "$rebuild_cmd"
      else
        log "WARNING: $rebuild_cmd absent; venv not rebuilt."
      fi
    fi
    for unit in libreqos-lqosd libreqos-bridge libreqos-scheduler libreqos-api; do
      [ "$unit" = "libreqos-bridge" ] && [ "$WITH_DATAPLANE" -eq 0 ] && continue
      if $SYSTEMCTL list-unit-files "${unit}.service" >/dev/null 2>&1; then
        if ! $SUDO $SYSTEMCTL restart "$unit"; then
          die "systemctl restart ${unit} failed; rolling back the install"
        fi
      fi
    done
  else
    log "NOTE: --no-restart; services and venv untouched."
  fi
  INSTALL_ACTIVE=0
  rm -f "$ROLLBACK_LIST"
  log "==> installed; backup at $BACKUP_DIR"
}

rollback_install() {
  local dst backup
  log "ROLLBACK: a mid-install failure was detected; restoring from $BACKUP_DIR"
  if [ -f "${ROLLBACK_LIST:-}" ]; then
    while IFS= read -r dst; do
      [ -n "$dst" ] || continue
      backup="$BACKUP_DIR/${dst#"$DEST_ROOT"}"
      if [ -e "$backup" ] || [ -L "$backup" ]; then
        $SUDO rm -rf "$dst"
        $SUDO mkdir -p "$(dirname "$dst")"
        $SUDO cp -a "$backup" "$dst"
      else
        $SUDO rm -rf "$dst"
      fi
    done < "$ROLLBACK_LIST"
    rm -f "$ROLLBACK_LIST"
  fi
}

clean_local() {
  rm -rf "$CP_WS/target" "$ENGINE_WS/target" "$BUILD_DIR" "$BIN_DIR/static2" "$SRC_ROOT/liblqos_python.so"
  log "==> cleaned target/, build/dev/, local venv, generated static2"
}

# Test-only seam: source for unit/behavior tests without running a command.
if [ "$DEV_TEST_MODE" -eq 1 ] && [ "${LQOS_BUILD_RUST_SOURCE_ONLY:-0}" = "1" ]; then
  return 0 2>/dev/null || exit 0
fi

case "$cmd" in
  check) check_host check ;;
  build)
    if [ "$DRY_RUN" -eq 1 ]; then build_plan; else check_host build; build_local; fi
    ;;

  install) install_local ;;
  clean) clean_local ;;
esac
