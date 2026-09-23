#!/bin/bash
# Build the LibreQoS Debian package on Debian hosts (Debian 13 "trixie" and
# later).
#
# This is the Debian front-end for build_dpkg.sh. It installs the Debian build
# prerequisites (which include bpftool, needed by the lqos_sys build) and then
# runs the shared packaging implementation, so the Ubuntu and Debian packages
# cannot drift apart.
#
# Run build_rust_debian.sh first if you also want the local src/bin install.
#
# Usage matches build_dpkg.sh: ./build_pkg_debian.sh [--nostamp]

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$SCRIPT_DIR" || exit 1
export LQOS_DISTRO=debian
# shellcheck source=build_prereqs.sh
source "$SCRIPT_DIR/build_prereqs.sh"
lqos_install_build_prerequisites
exec "$SCRIPT_DIR/build_dpkg.sh" "$@"
