#!/bin/bash

set -euo pipefail

HOTFIX_REPO_URL="${HOTFIX_REPO_URL:-https://repo.libreqos.com}"
HOTFIX_REPO_DIST="${HOTFIX_REPO_DIST:-noble}"
HOTFIX_REPO_COMPONENT="${HOTFIX_REPO_COMPONENT:-main}"
HOTFIX_KEY_URL="${HOTFIX_KEY_URL:-${HOTFIX_REPO_URL}/keys/libreqos-archive-keyring.gpg}"
HOTFIX_KEYRING_PATH="${HOTFIX_KEYRING_PATH:-/usr/share/keyrings/libreqos-archive-keyring.gpg}"
HOTFIX_APT_SOURCE_PATH="${HOTFIX_APT_SOURCE_PATH:-/etc/apt/sources.list.d/libreqos-systemd-hotfix.list}"
HOTFIX_APT_PREFERENCES_PATH="${HOTFIX_APT_PREFERENCES_PATH:-/etc/apt/preferences.d/libreqos-systemd-hotfix}"
HOTFIX_APT_PIN_ORIGIN="${HOTFIX_APT_PIN_ORIGIN:-LibreQoS}"
HOTFIX_APT_PIN_LABEL="${HOTFIX_APT_PIN_LABEL:-LibreQoS}"
HOTFIX_PACKAGE_VERSION="${HOTFIX_PACKAGE_VERSION:-255.4-1ubuntu8.14+libreqos2}"
SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS="${SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS:-255.4-1ubuntu8 255.4-1ubuntu8.*}"
HOTFIX_MARKER="${HOTFIX_MARKER:-/opt/libreqos/src/.systemd_hotfix_installed}"
HOTFIX_SKIP_REBOOT_PROMPT="${HOTFIX_SKIP_REBOOT_PROMPT:-0}"

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
  bootstrap     Configure the LibreQoS APT repo and package pin for this host
  download      Download the hotfix package set into a temporary directory via APT
  install       Configure the LibreQoS APT repo and install the hotfix package set
  packages      Print the package names managed by the hotfix on this host
  urls          Print the repo bootstrap URLs used for this host

Environment:
  HOTFIX_REPO_URL                  LibreQoS APT repository URL
  HOTFIX_REPO_DIST                 APT distribution, defaults to noble
  HOTFIX_REPO_COMPONENT            APT component, defaults to main
  HOTFIX_KEY_URL                   Public key URL for the LibreQoS APT repository
  HOTFIX_KEYRING_PATH              Destination keyring path installed on the host
  HOTFIX_APT_SOURCE_PATH           Destination apt source list path installed on the host
  HOTFIX_APT_PREFERENCES_PATH      Destination apt pin file installed on the host
  HOTFIX_APT_PIN_ORIGIN            Expected Release Origin field, defaults to LibreQoS
  HOTFIX_APT_PIN_LABEL             Expected Release Label field, defaults to LibreQoS
  HOTFIX_PACKAGE_VERSION           Backported package version to install
  SUPPORTED_UBUNTU_SYSTEMD_VERSION_GLOBS Space-separated stock Ubuntu version globs eligible for replacement
  HOTFIX_MARKER                    Marker file written after install
  HOTFIX_SKIP_REBOOT_PROMPT        Set to 1 to suppress the reboot prompt after install
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

resolved_hotfix_package_names() {
    local package

    for package in "${HOTFIX_CORE_PACKAGES[@]}"; do
        printf '%s\n' "$package"
    done

    for package in "${HOTFIX_OPTIONAL_PACKAGES[@]}"; do
        if package_is_installed "$package"; then
            printf '%s\n' "$package"
        fi
    done
}

resolved_hotfix_package_specs() {
    local package
    while IFS= read -r package; do
        printf '%s=%s\n' "$package" "$HOTFIX_PACKAGE_VERSION"
    done < <(resolved_hotfix_package_names)
}

joined_hotfix_packages() {
    local packages=()
    local package

    while IFS= read -r package; do
        packages+=("$package")
    done < <(resolved_hotfix_package_names)

    printf '%s\n' "${packages[*]}"
}

render_apt_source() {
    printf 'deb [signed-by=%s] %s %s %s\n' \
        "$HOTFIX_KEYRING_PATH" \
        "$HOTFIX_REPO_URL" \
        "$HOTFIX_REPO_DIST" \
        "$HOTFIX_REPO_COMPONENT"
}

render_apt_preferences() {
    cat <<EOF
Package: $(joined_hotfix_packages)
Pin: release o=${HOTFIX_APT_PIN_ORIGIN},l=${HOTFIX_APT_PIN_LABEL},n=${HOTFIX_REPO_DIST}
Pin-Priority: 1001
EOF
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

ensure_applicable_host() {
    local version
    version="$(current_systemd_version)"

    is_supported_os || fail "Host is not Ubuntu 24.04 Noble. Hotfix not applicable."
    [[ -n "$version" ]] || fail "systemd is not installed via dpkg query. Hotfix not applicable."
    uses_systemd_networkd || fail "systemd-networkd is not enabled or active. Hotfix not applicable."
}

print_urls() {
    printf 'repo=%s\n' "$HOTFIX_REPO_URL"
    printf 'key=%s\n' "$HOTFIX_KEY_URL"
}

print_packages() {
    resolved_hotfix_package_names
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

bootstrap_repo() {
    require_command curl
    require_command apt-get

    ensure_applicable_host

    run_as_root install -d -m 755 \
        "$(dirname "$HOTFIX_KEYRING_PATH")" \
        "$(dirname "$HOTFIX_APT_SOURCE_PATH")" \
        "$(dirname "$HOTFIX_APT_PREFERENCES_PATH")"

    curl -fsSL "$HOTFIX_KEY_URL" | run_as_root tee "$HOTFIX_KEYRING_PATH" >/dev/null
    run_as_root chmod 644 "$HOTFIX_KEYRING_PATH"

    render_apt_source | run_as_root tee "$HOTFIX_APT_SOURCE_PATH" >/dev/null
    render_apt_preferences | run_as_root tee "$HOTFIX_APT_PREFERENCES_PATH" >/dev/null
    run_as_root chmod 644 "$HOTFIX_APT_SOURCE_PATH" "$HOTFIX_APT_PREFERENCES_PATH"

    run_as_root apt-get update
    log "Configured LibreQoS APT hotfix repository: $HOTFIX_REPO_URL"
}

download_bundle() {
    local workdir

    require_command apt-get
    bootstrap_repo

    workdir="$(mktemp -d /tmp/libreqos-systemd-hotfix.XXXXXX)"
    (
        cd "$workdir"
        while IFS= read -r package; do
            apt-get download "$package"
        done < <(resolved_hotfix_package_specs)
    )

    printf '%s\n' "$workdir"
}

write_marker() {
    local package
    {
        printf 'installed_at=%s\n' "$(date -Iseconds)"
        printf 'package_version=%s\n' "$HOTFIX_PACKAGE_VERSION"
        printf 'repo_url=%s\n' "$HOTFIX_REPO_URL"
        printf 'key_url=%s\n' "$HOTFIX_KEY_URL"
        printf 'apt_source_path=%s\n' "$HOTFIX_APT_SOURCE_PATH"
        printf 'apt_preferences_path=%s\n' "$HOTFIX_APT_PREFERENCES_PATH"
        printf 'systemd_version=%s\n' "$(current_systemd_version)"
        while IFS= read -r package; do
            printf 'package_name=%s\n' "$package"
            printf 'package_spec=%s=%s\n' "$package" "$HOTFIX_PACKAGE_VERSION"
        done < <(resolved_hotfix_package_names)
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
    local package_specs=()
    local package

    require_command apt-get
    ensure_applicable_host
    bootstrap_repo

    while IFS= read -r package; do
        package_specs+=("$package")
    done < <(resolved_hotfix_package_specs)

    run_as_root apt-get install -y "${package_specs[@]}"
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
        bootstrap)
            bootstrap_repo
            ;;
        download)
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
