#!/bin/bash
# Shared build-prerequisite handling for the LibreQoS build scripts.
#
# build_rust.sh and build_dpkg.sh are the source of truth for the build and
# package flows. This helper centralizes the per-distribution package list so
# the Ubuntu and Debian paths cannot drift apart.

# Prints the distribution id used to select build packages.
# LQOS_DISTRO overrides /etc/os-release, which is useful on derivatives and in
# tests.
lqos_detect_distro() {
    if [ -n "${LQOS_DISTRO:-}" ]; then
        printf '%s\n' "${LQOS_DISTRO,,}"
        return 0
    fi
    if [ -r /etc/os-release ]; then
        local id
        id=$(awk -F= '$1 == "ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
        if [ -n "$id" ]; then
            printf '%s\n' "${id,,}"
            return 0
        fi
    fi
    printf 'unknown\n'
}

# Prints the build prerequisite packages for a distribution id.
# Returns non-zero for distributions without a known package set.
lqos_build_packages() {
    case "$1" in
        ubuntu)
            # linux-tools-common and linux-tools-<kernel> provide bpftool and
            # perf on Ubuntu.
            printf '%s\n' python3-pip python3-venv clang gcc gcc-multilib llvm \
                libelf-dev git nano curl screen pkg-config make \
                linux-tools-common "linux-tools-$(uname -r)" libbpf-dev libssl-dev
            ;;
        debian)
            # Debian ships bpftool and perf as standalone packages instead of
            # the Ubuntu linux-tools-* names.
            printf '%s\n' python3-pip python3-venv clang gcc gcc-multilib llvm \
                libelf-dev git nano curl screen pkg-config make \
                bpftool linux-perf libbpf-dev libssl-dev
            ;;
        *)
            return 1
            ;;
    esac
}

# Installs the build prerequisites for the current or overridden distribution.
lqos_install_build_prerequisites() {
    local distro packages
    distro=$(lqos_detect_distro)
    if ! packages=$(lqos_build_packages "$distro"); then
        echo "Unsupported distribution '$distro' for automatic prerequisite install."
        echo "Install the LibreQoS build prerequisites manually: clang, llvm, bpftool, make, pkg-config, libelf-dev, libssl-dev."
        return 0
    fi
    echo "Installing LibreQoS build prerequisites for $distro"
    # shellcheck disable=SC2086
    sudo apt-get install -y $packages
}
