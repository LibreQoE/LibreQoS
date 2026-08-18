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
        domain_exists "$domain" && die "lab domain already exists: $domain (run ./radius-harness/lab down first)"
    done
    for network in "$CONTROL_NETWORK" "$ACCESS_NETWORK" "$TRANSIT_NETWORK"; do
        network_exists "$network" && die "lab network already exists: $network (inspect and remove it before starting this lab)"
    done
}

management_ip() {
    local mac=$1
    virsh_lab net-dhcp-leases "$MANAGEMENT_NETWORK" | awk -v wanted_mac="$mac" 'tolower($0) ~ tolower(wanted_mac) { expires = $1 " " $2; if (expires > latest) { latest = expires; split($5, address, "/"); ip = address[1] } } END { print ip }'
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

lab_ssh() { local host=$1; shift; ssh -i "$RUN_DIR/id_ed25519" -o IdentitiesOnly=yes -o BatchMode=yes -o UserKnownHostsFile="$RUN_DIR/known_hosts" -o StrictHostKeyChecking=accept-new "lab@$host" "$@"; }
lab_scp() { local source=$1 host=$2 destination=$3; scp -i "$RUN_DIR/id_ed25519" -o IdentitiesOnly=yes -p -o BatchMode=yes -o UserKnownHostsFile="$RUN_DIR/known_hosts" -o StrictHostKeyChecking=accept-new -r "$source" "lab@$host:$destination"; }

safe_remove() {
    local path=$1
    [[ $path == "$LAB_ROOT"/* ]] || die "refusing to remove path outside lab root: $path"
    rm -rf -- "$path"
}
