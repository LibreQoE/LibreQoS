#!/usr/bin/env bash
# LibreQoS v3 monorepo developer bootstrap.
#
# Kept for v2 muscle memory. The engine (rust/libreqos_dpdk) and the shared
# types (rust/libreqos_units) are IN THIS REPO; there is no separate engine checkout,
# no cross-repo pin, and no analytics_ipc lockstep. This is a thin alias for
# `build_rust.sh build`.
set -euo pipefail
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
exec bash "${SRC_DIR}/build_rust.sh" build "$@"
