#!/usr/bin/env bash
set -euo pipefail

LAB_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
REPO_ROOT=${LIBREQOS_REPO_ROOT:-$(cd -- "$LAB_ROOT/.." && pwd)}
RUNTIME_DIR="$LAB_ROOT/artifacts/runtime"
IMAGE_DIR="$LAB_ROOT/images"
RUN_DIR="$LAB_ROOT/run"
LIBVIRT_URI=${LIBVIRT_URI:-qemu:///system}
MANAGEMENT_NETWORK=default

readonly LQOS_DOMAIN=radius-lqos
readonly ROUTER_DOMAIN=radius-routeros
readonly RADIUS_DOMAIN=radius-freeradius
readonly CLIENT_DOMAIN=radius-client
readonly CONTROL_NETWORK=radius-control
readonly ACCESS_NETWORK=radius-access
readonly TRANSIT_NETWORK=radius-transit

die() { echo "error: $*" >&2; exit 1; }
note() { echo "==> $*"; }
require_command() { command -v "$1" >/dev/null || die "required command not found: $1"; }
require_lab_secret() { local secret_name=$1; [[ -n ${!secret_name:-} ]] || die "set $secret_name in the environment"; }
virsh_lab() { virsh -c "$LIBVIRT_URI" "$@"; }
domain_exists() { virsh_lab dominfo "$1" >/dev/null 2>&1; }
network_exists() { virsh_lab net-info "$1" >/dev/null 2>&1; }

ensure_lab_stopped() {
    for domain in "$LQOS_DOMAIN" "$ROUTER_DOMAIN" "$RADIUS_DOMAIN" "$CLIENT_DOMAIN"; do
        if domain_exists "$domain"; then
            die "lab domain already exists: $domain (run ./radius-harness/lab down first)"
        fi
    done
    for network in "$CONTROL_NETWORK" "$ACCESS_NETWORK" "$TRANSIT_NETWORK"; do
        if network_exists "$network"; then
            die "lab network already exists: $network (inspect and remove it before starting this lab)"
        fi
    done
}

# Prints the guest OS recorded for this lab: LAB_GUEST_OS, the images/guest-os
# marker written by init and up, or the ubuntu default.
resolve_guest_os() {
    local guest_os=${LAB_GUEST_OS:-}
    if [[ -z $guest_os && -f $IMAGE_DIR/guest-os ]]; then
        guest_os=$(<"$IMAGE_DIR/guest-os")
    fi
    printf '%s\n' "${guest_os:-ubuntu}"
}

# Prints "image_url|checksum|checksum_tool|os_variant|os_release" for a guest
# OS. Returns non-zero for unknown guest operating systems.
guest_os_facts() {
    case "$1" in
        ubuntu)
            printf '%s|%s|%s|%s|%s\n' "$UBUNTU_IMAGE_URL" "$UBUNTU_IMAGE_SHA256" sha256sum ubuntu24.04 'ubuntu 24.04'
            ;;
        debian)
            printf '%s|%s|%s|%s|%s\n' "$DEBIAN_IMAGE_URL" "$DEBIAN_IMAGE_SHA512" sha512sum debian13 'debian 13'
            ;;
        *)
            return 1
            ;;
    esac
}

# Prints the /etc/os-release "ID VERSION_ID" string expected for a guest OS.
guest_expected_os_release() {
    local facts
    facts=$(guest_os_facts "$1") || return 1
    printf '%s\n' "${facts##*|}"
}

management_ip() {
    local mac=$1 now line day time mac_field proto ip_field rest best_ip= best_expires=0 epoch
    now=$(date +%s)
    while IFS= read -r line; do
        read -r day time mac_field proto ip_field rest <<<"$line" || continue
        [[ ${mac_field,,} == "${mac,,}" ]] || continue
        [[ ${proto,,} == "ipv4" ]] || continue
        epoch=$(date -d "$day $time" +%s 2>/dev/null) || continue
        if (( epoch <= now )); then
            continue
        fi
        if (( epoch > best_expires )); then
            best_expires=$epoch
            best_ip=${ip_field%%/*}
        fi
    done < <(virsh_lab net-dhcp-leases "$MANAGEMENT_NETWORK")
    printf '%s\n' "$best_ip"
}

wait_for_management_ip() {
    local mac=$1 ip
    for _ in $(seq 1 60); do
        ip=$(management_ip "$mac")
        [[ -n $ip ]] && { printf '%s\n' "$ip"; return 0; }
        sleep 2
    done
    die "timed out waiting for DHCP lease for $mac"
}

wait_for_ssh() {
    local host=$1
    for _ in $(seq 1 60); do
        if ssh -i "$RUN_DIR/id_ed25519" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=2 -o UserKnownHostsFile="$RUN_DIR/known_hosts" -o StrictHostKeyChecking=accept-new "lab@$host" true >/dev/null 2>&1; then return 0; fi
        sleep 2
    done
    die "timed out waiting for SSH on $host"
}

wait_for_cloud_init() {
    local host=$1 output
    if output=$(lab_ssh "$host" 'sudo timeout 900 cloud-init status --wait --long' 2>&1); then
        return 0
    fi
    printf '%s\n' "$output"
    die "cloud-init did not finish successfully on $host"
}

wait_for_guest_ready() {
    local mac=$1 ip
    ip=$(wait_for_management_ip "$mac")
    wait_for_ssh "$ip"
    wait_for_cloud_init "$ip"
    printf '%s\n' "$ip"
}

assert_guest_os() {
    local host=$1 guest_os=$2 expected actual
    expected=$(guest_expected_os_release "$guest_os") || die "unsupported GUEST_OS '$guest_os'"
    actual=$(lab_ssh "$host" '. /etc/os-release && printf "%s %s" "$ID" "$VERSION_ID"')
    [[ $actual == "$expected" ]] || die "LibreQoS guest is '$actual', expected '$expected'"
}

lab_ssh() { local host=$1; shift; ssh -i "$RUN_DIR/id_ed25519" -o IdentitiesOnly=yes -o BatchMode=yes -o UserKnownHostsFile="$RUN_DIR/known_hosts" -o StrictHostKeyChecking=accept-new "lab@$host" "$@"; }
lab_scp() { local source=$1 host=$2 destination=$3; scp -i "$RUN_DIR/id_ed25519" -o IdentitiesOnly=yes -p -o BatchMode=yes -o UserKnownHostsFile="$RUN_DIR/known_hosts" -o StrictHostKeyChecking=accept-new -r "$source" "lab@$host:$destination"; }

safe_remove() {
    local path=$1
    [[ $path == "$LAB_ROOT"/* ]] || die "refusing to remove path outside lab root: $path"
    rm -rf -- "$path"
}
