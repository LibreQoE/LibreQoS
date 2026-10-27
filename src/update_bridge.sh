#!/bin/bash
#
# Fetch and install the LibreQoS DATAPLANE from the apt archive.
#
# The public/control-plane deb ships NO dataplane payload (docs/
# distribution_split_card.md §4): the DPDK bridge binary, its runtime closure,
# the EAL PMDs, the bridge systemd unit/wrapper and the privilege grants ship
# as the `libreqos-dataplane` package. This script is the install/update hook
# (docs/apt_repo_distribution_card.md §3): it configures the archive, then
# installs/updates the package — integrity via the repo's GPG keyring instead
# of a bare tarball. It is GRACEFUL on an unreachable repo: it prints NOT
# FETCHED and exits 0, and the bridge unit stays inactive (fail-closed; the
# control plane still runs).
#
# Usage: update_bridge.sh [--url URL] [--suite SUITE] [--no-restart]
#
# Environment:
#   LQOS_REPO_URL                 archive base URL
#   LQOS_REPO_SUITE               suite (stable | beta | archive)
#   LQOS_REPO_KEYRING             the archive signing keyring (signed-by)
#   LQOS_DATAPLANE_RESTART_SERVICE 1 to restart the bridge (default), 0 to skip

set -euo pipefail

REPO_URL="${LQOS_REPO_URL:-https://packages.libreqos.com}"
KEYRING="${LQOS_REPO_KEYRING:-/usr/share/keyrings/libreqos-packages-keyring.gpg}"
SOURCE_LIST="/etc/apt/sources.list.d/libreqos.list"

# Derive the suite from an existing v3 source (including a .v3-disabled one
# left by the rollback), never hardcode beta/stable. LQOS_REPO_SUITE wins.
detect_suite() {
  local dir f suite
  dir="$(dirname "$SOURCE_LIST")"
  for f in "$SOURCE_LIST" "$dir"/*.list "$dir"/*.sources "$dir"/*.v3-disabled; do
    [ -r "$f" ] || continue
    grep -q 'packages\.libreqos\.com' "$f" 2>/dev/null || continue
    case "$f" in
      *.sources*)
        suite="$(sed -n 's/^[[:space:]]*Suites:[[:space:]]*//p' "$f" | head -n1 | awk '{print $1}')"
        ;;
      *)
        suite="$(sed -n 's|^[[:space:]]*deb[[:space:]].*packages\.libreqos\.com[[:space:]]\+\([^[:space:]]*\).*|\1|p' "$f" | head -n1)"
        ;;
    esac
    if [ -n "${suite:-}" ]; then printf '%s' "$suite"; return 0; fi
  done
  printf '%s' "beta"
}
REPO_SUITE="${LQOS_REPO_SUITE:-$(detect_suite)}"
if [ "$REPO_SUITE" = "stable" ]; then
  echo "dataplane: WARNING: packages.libreqos.com stable is EMPTY before GA; the fetch will fail. Use LQOS_REPO_SUITE=beta (or point libreqos.list at beta; the .12 lab uses beta). Set stable only at GA." >&2
fi
RESTART_SERVICE="${LQOS_DATAPLANE_RESTART_SERVICE:-1}"
PACKAGE="libreqos-dataplane"
BRIDGE_BIN="/usr/libexec/libreqos/bridge_rs"

usage() {
  cat <<EOF
Usage: $0 [--url URL] [--suite SUITE] [--no-restart]

Options:
  --url URL       Archive base URL (default: $REPO_URL)
  --suite SUITE   Archive suite (default: $REPO_SUITE)
  --no-restart    Do not restart the bridge service

Environment:
  LQOS_REPO_URL / LQOS_REPO_SUITE / LQOS_REPO_KEYRING
  LQOS_DATAPLANE_RESTART_SERVICE 1 to restart the bridge (default), 0 to skip
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url)
      [[ $# -ge 2 ]] || { echo "Error: --url requires an argument" >&2; exit 2; }
      REPO_URL="$2"; shift 2 ;;
    --suite)
      [[ $# -ge 2 ]] || { echo "Error: --suite requires an argument" >&2; exit 2; }
      REPO_SUITE="$2"; shift 2 ;;
    --no-restart) RESTART_SERVICE=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Error: unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

not_fetched() {
  echo "dataplane: NOT FETCHED — $1; the bridge stays inactive" >&2
  exit 0
}

# The archive must be signed; without the keyring we cannot trust a fetch.
[[ -e "$KEYRING" ]] || not_fetched "the LibreQoS archive keyring ($KEYRING) is absent"

# Configure the archive once (an operator/other package may have done it).
if [[ ! -e "$SOURCE_LIST" ]]; then
  printf 'deb [signed-by=%s] %s %s main\n' "$KEYRING" "$REPO_URL" "$REPO_SUITE" > "$SOURCE_LIST"
  echo "dataplane: configured the LibreQoS archive ($REPO_URL $REPO_SUITE)"
fi

# Refresh ONLY our list (never the whole system's), then install/update.
if ! apt-get update \
      -o Dir::Etc::sourcelist="sources.list.d/libreqos.list" \
      -o Dir::Etc::sourceparts="-" \
      -o APT::Get::List-Cleanup="0" >/dev/null 2>&1; then
  not_fetched "the LibreQoS archive is unreachable ($REPO_URL)"
fi

if ! DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$PACKAGE" >/dev/null 2>&1; then
  not_fetched "$PACKAGE is unavailable from the $REPO_SUITE suite"
fi

# A successful install leaves the binary in place; enable-on-success is
# unchanged (the package's postinst also enables it).
if [[ -x "$BRIDGE_BIN" ]]; then
  /bin/systemctl enable libreqos-bridge.service 2>/dev/null || true
  if [[ "$RESTART_SERVICE" == "1" ]]; then
    /bin/systemctl restart libreqos-bridge.service 2>/dev/null || true
  else
    echo "Skipping the bridge service restart."
  fi
else
  not_fetched "$PACKAGE installed but $BRIDGE_BIN is missing"
fi

echo "dataplane: FETCHED (apt: $PACKAGE from $REPO_SUITE) -> $BRIDGE_BIN"
