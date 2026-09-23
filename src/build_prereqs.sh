#!/bin/bash
# Shared build-prerequisite handling for the LibreQoS build scripts.
#
# build_rust.sh and build_dpkg.sh are the source of truth for the build and
# package flows. This helper centralizes the per-distribution package list so
# the Ubuntu and Debian paths cannot drift apart.

# Build packages shared by every supported distribution.
LQOS_COMMON_BUILD_PACKAGES=(
    python3-pip python3-venv clang gcc gcc-multilib llvm libelf-dev git nano
    curl screen pkg-config make libbpf-dev libssl-dev
)

# Prints the distribution id used to select build packages.
# LQOS_DISTRO overrides /etc/os-release, and ID_LIKE covers derivatives such as
# Linux Mint and Pop!_OS.
lqos_detect_distro() {
    local id id_like
    if [ -n "${LQOS_DISTRO:-}" ]; then
        printf '%s\n' "${LQOS_DISTRO,,}"
        return 0
    fi
    if [ -r /etc/os-release ]; then
        id=$(awk -F= '$1 == "ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
        case "$id" in
            ubuntu|debian)
                printf '%s\n' "$id"
                return 0
                ;;
        esac
        id_like=$(awk -F= '$1 == "ID_LIKE" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
        case " $id_like " in
            *" ubuntu "*)
                printf 'ubuntu\n'
                return 0
                ;;
            *" debian "*)
                printf 'debian\n'
                return 0
                ;;
        esac
        if [ -n "$id" ]; then
            printf '%s\n' "${id,,}"
            return 0
        fi
    fi
    printf 'unknown\n'
}

# Prints the distribution-specific build packages.
# Returns non-zero for distributions without a known package set.
lqos_distro_build_extras() {
    case "$1" in
        ubuntu)
            # linux-tools-common and linux-tools-<kernel> provide bpftool and
            # perf on Ubuntu.
            printf '%s\n' linux-tools-common "linux-tools-$(uname -r)"
            ;;
        debian)
            # Debian ships bpftool and perf as standalone packages instead of
            # the Ubuntu linux-tools-* names.
            printf '%s\n' bpftool linux-perf
            ;;
        *)
            return 1
            ;;
    esac
}

# Installs the build prerequisites for the current or overridden distribution.
# Set LQOS_SKIP_PREREQS=1 to skip the package installation.
# Side effects: refreshes apt package lists, installs packages, and prepends
# /usr/sbin to PATH so the build can find bpftool on Debian.
lqos_install_build_prerequisites() {
    local distro extras

    # Debian installs bpftool in /usr/sbin, which is not on the default
    # non-root PATH there, and lqos_sys invokes bpftool by name during the
    # build. Apply this even when the install is skipped.
    export PATH="/usr/sbin:$PATH"

    if [ "${LQOS_SKIP_PREREQS:-0}" = "1" ]; then
        echo "Skipping build prerequisite installation (LQOS_SKIP_PREREQS=1)"
        return 0
    fi
    distro=$(lqos_detect_distro)

    if ! extras=$(lqos_distro_build_extras "$distro"); then
        echo "Unsupported distribution '$distro' for automatic prerequisite install."
        echo "Install these packages with your distribution's package manager:"
        printf '%s\n' "${LQOS_COMMON_BUILD_PACKAGES[@]}" | sed 's/^/  /'
        echo "Also install bpftool (required to build lqos_sys) and, optionally, perf."
        echo "Or install them yourself and re-run with LQOS_SKIP_PREREQS=1."
        return 1
    fi

    echo "Installing LibreQoS build prerequisites for $distro"
    sudo apt-get update || echo "Warning: apt-get update failed; continuing with cached package lists"
    sudo apt-get install -y "${LQOS_COMMON_BUILD_PACKAGES[@]}" || return 1

    # Kernel tools packages track the running kernel and can be unavailable on
    # custom or HWE kernels. Install each independently so a missing
    # kernel-pinned package cannot block the rest.
    while IFS= read -r package; do
        if ! sudo apt-get install -y "$package"; then
            echo "Warning: failed to install: $package"
        fi
    done <<<"$extras"
    if ! command -v bpftool >/dev/null 2>&1; then
        echo "bpftool was not found; trying the standalone package."
        sudo apt-get install -y bpftool || true
    fi
    if ! command -v bpftool >/dev/null 2>&1; then
        echo "Warning: bpftool is still missing; lqos_sys cannot build without it."
    fi
}
