#!/bin/bash

set -euo pipefail

HOTFIX_BASE_URL="https://download.libreqos.com"
HOTFIX_PACKAGE_VERSION="${HOTFIX_PACKAGE_VERSION:-255.4-1ubuntu8.12+libreqos1}"
SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS="${SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS:-255.4-1ubuntu8 255.4-1ubuntu8.*}"
HOTFIX_MARKER="${HOTFIX_MARKER:-/opt/libreqos/src/.systemd_hotfix_installed}"
HOTFIX_SKIP_REBOOT_PROMPT="${HOTFIX_SKIP_REBOOT_PROMPT:-0}"
ARCH="${ARCH:-amd64}"

HOTFIX_CORE_PACKAGES=(
  "libsystemd0"
  "libsystemd-shared"
  "libudev1"
  "systemd-dev"
  "systemd"
  "systemd-sysv"
  "systemd-resolved"
  "systemd-timesyncd"
  "udev"
)

HOTFIX_OPTIONAL_PACKAGES=(
  "libpam-systemd"
  "libnss-systemd"
  "libnss-resolve"
  "libnss-myhostname"
)

usage() {
    cat <<EOF
Usage: $0 <command>

Commands:
  status        Show whether this host should be offered the Noble systemd hotfix
  should-offer  Exit 0 when the hotfix should be offered on this host
  download      Download the hotfix bundle into a temporary directory
  install       Download and install the hotfix bundle, then offer to schedule a reboot
  packages      Print the package filenames expected for this host
  urls          Print the expected package URLs for this host

Environment:
  HOTFIX_PACKAGE_VERSION           Backported package version suffix
  SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS Space-separated stock Ubuntu version globs eligible for replacement
  HOTFIX_MARKER                    Marker file written after install
  HOTFIX_SKIP_REBOOT_PROMPT        Set to 1 to suppress the reboot prompt after install
  ARCH                             Debian architecture suffix, defaults to amd64
EOF
}

log() {
    printf '%s\n' "$*"
}

fail() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "Required command not found: $1"
}

run_as_root() {
    if [[ "$(id -u)" -eq 0 ]]; then
        "$@"
        return
    fi

    require_command sudo
    sudo "$@"
}

has_tty_prompt() {
    [[ -r /dev/tty && -w /dev/tty ]]
}

prompt_yes_no() {
    local prompt="$1"
    local default_answer="$2"
    local reply

    while true; do
        printf '%s ' "$prompt" >/dev/tty
        IFS= read -r reply </dev/tty || return 1

        if [[ -z "$reply" ]]; then
            reply="$default_answer"
        fi

        case "$reply" in
            [Yy]|[Yy][Ee][Ss])
                return 0
                ;;
            [Nn]|[Nn][Oo])
                return 1
                ;;
            *)
                printf 'Please answer y or n.\n' >/dev/tty
                ;;
        esac
    done
}

current_systemd_version() {
    dpkg-query -W -f='${Version}\n' systemd 2>/dev/null || true
}

package_is_installed() {
    local package="$1"
    dpkg-query -W -f='${db:Status-Abbrev}\n' "$package" 2>/dev/null | grep -q '^ii'
}

package_arch_suffix() {
    local package="$1"

    case "$package" in
        systemd-dev)
            printf 'all\n'
            ;;
        *)
            printf '%s\n' "$ARCH"
            ;;
    esac
}

package_filename() {
    local package="$1"
    printf '%s_%s_%s.deb\n' \
        "$package" \
        "$HOTFIX_PACKAGE_VERSION" \
        "$(package_arch_suffix "$package")"
}

resolved_hotfix_packages() {
    local package

    for package in "${HOTFIX_CORE_PACKAGES[@]}"; do
        package_filename "$package"
    done

    for package in "${HOTFIX_OPTIONAL_PACKAGES[@]}"; do
        if package_is_installed "$package"; then
            package_filename "$package"
        fi
    done
}

is_supported_os() {
    [[ -r /etc/os-release ]] || return 1
    # shellcheck disable=SC1091
    . /etc/os-release
    [[ "${ID:-}" == "ubuntu" && "${VERSION_ID:-}" == "24.04" && "${VERSION_CODENAME:-}" == "noble" ]]
}

is_installed_hotfix() {
    local version
    version="$(current_systemd_version)"
    [[ "$version" == *"+libreqos"* ]]
}

is_supported_stock_version() {
    local version supported
    version="$(current_systemd_version)"
    for supported in $SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS; do
        [[ "$version" == $supported ]] && return 0
    done
    return 1
}

uses_systemd_networkd() {
    local enabled_state active_state

    enabled_state="$(systemctl is-enabled systemd-networkd 2>/dev/null || true)"
    active_state="$(systemctl is-active systemd-networkd 2>/dev/null || true)"

    [[ "$enabled_state" == "enabled" || "$enabled_state" == "static" || "$active_state" == "active" ]]
}

print_urls() {
    local package
    while IFS= read -r package; do
        printf '%s/%s\n' "$HOTFIX_BASE_URL" "$package"
    done < <(resolved_hotfix_packages)
}

print_packages() {
    resolved_hotfix_packages
}

status() {
    local version
    version="$(current_systemd_version)"

    if ! is_supported_os; then
        log "Host is not Ubuntu 24.04 Noble. Hotfix not applicable."
        return 1
    fi

    if [[ -z "$version" ]]; then
        log "systemd is not installed via dpkg query. Hotfix not applicable."
        return 1
    fi

    if ! uses_systemd_networkd; then
        log "systemd-networkd is not enabled or active. Hotfix not applicable."
        return 1
    fi

    if is_installed_hotfix; then
        log "LibreQoS hotfix already installed: $version"
        return 1
    fi

    if is_supported_stock_version; then
        log "Hotfix should be offered. Installed systemd version: $version"
        return 0
    fi

    log "Installed systemd version is not in the supported replacement list: $version"
    return 1
}

download_bundle() {
    local workdir package
    require_command curl
    workdir="$(mktemp -d /tmp/libreqos-systemd-hotfix.XXXXXX)"
    while IFS= read -r package; do
        curl -fL --retry 3 --output "${workdir}/${package}" "${HOTFIX_BASE_URL}/${package}"
    done < <(resolved_hotfix_packages)
    printf '%s\n' "$workdir"
}

write_marker() {
    local package
    {
        printf 'installed_at=%s\n' "$(date -Iseconds)"
        printf 'package_version=%s\n' "$HOTFIX_PACKAGE_VERSION"
        printf 'base_url=%s\n' "$HOTFIX_BASE_URL"
        printf 'systemd_version=%s\n' "$(current_systemd_version)"
        while IFS= read -r package; do
            printf 'package_file=%s\n' "$package"
            printf 'package_url=%s/%s\n' "$HOTFIX_BASE_URL" "$package"
        done < <(resolved_hotfix_packages)
    } | run_as_root tee "$HOTFIX_MARKER" >/dev/null
}

offer_reboot() {
    if [[ "$HOTFIX_SKIP_REBOOT_PROMPT" == "1" ]]; then
        log "Reboot required before validating networkd behavior."
        return
    fi

    if ! has_tty_prompt; then
        log "Reboot required before validating networkd behavior."
        return
    fi

    if prompt_yes_no "Schedule a reboot in 1 minute now? [y/N]" "n"; then
        require_command shutdown
        run_as_root shutdown -r +1 "LibreQoS systemd hotfix installed; reboot scheduled."
        log "Reboot scheduled in 1 minute."
        return
    fi

    log "Reboot not scheduled. Reboot before validating networkd behavior."
}

install_bundle() {
    local workdir package_paths=() package

    status >/dev/null || fail "Hotfix is not applicable on this host."
    require_command apt-get

    workdir="$(download_bundle)"
    while IFS= read -r package; do
        package_paths+=("${workdir}/${package}")
    done < <(resolved_hotfix_packages)

    run_as_root apt-get install -y "${package_paths[@]}"
    write_marker
    log "Hotfix installed."
    offer_reboot
}

main() {
    local command="${1:-}"

    case "$command" in
        status)
            status
            ;;
        should-offer)
            status >/dev/null
            ;;
        download)
            status >/dev/null || fail "Hotfix is not applicable on this host."
            download_bundle
            ;;
        packages)
            print_packages
            ;;
        install)
            install_bundle
            ;;
        urls)
            print_urls
            ;;
        *)
            usage
            exit 1
            ;;
    esac
}

main "$@"
