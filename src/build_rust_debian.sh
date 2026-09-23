#!/bin/bash
# Build LibreQoS on Debian hosts (Debian 13 "trixie" and later).
#
# This is the Debian front-end for build_rust.sh. It selects the Debian
# prerequisite package set (bpftool and linux-perf replace the Ubuntu
# linux-tools-* packages) and runs the shared build implementation, so the
# Ubuntu and Debian paths cannot drift apart.
#
# Usage matches build_rust.sh: ./build_rust_debian.sh [--fast]

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$SCRIPT_DIR" || exit 1
export LQOS_DISTRO=debian
exec "$SCRIPT_DIR/build_rust.sh" "$@"
