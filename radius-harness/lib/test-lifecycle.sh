#!/usr/bin/env bash
set -euo pipefail

LAB_ROOT=$(cd -- "$(dirname -- "$0")/.." && pwd)
source "$LAB_ROOT/lib/common.sh"

journal_cursor() { local lqos_ip=$1; lab_ssh "$lqos_ip" "sudo journalctl -u radius-lqosd -n 0 --show-cursor --no-pager | sed -n 's/^-- cursor: //p'"; }

wait_for_log() {
    local lqos_ip=$1 cursor=$2 pattern=$3 description=$4
    for _ in $(seq 1 20); do
        lab_ssh "$lqos_ip" "sudo journalctl -u radius-lqosd --after-cursor='$cursor' --no-pager | grep -q -- '$pattern'" && return 0
        sleep 2
    done
    die "$description"
}

dynamic_circuit_count_is() { local lqos_ip=$1 expected_count=$2; lab_ssh "$lqos_ip" "sudo python3 -c 'import json, os, sys; path = \"/opt/libreqos/src/dynamic_circuits.json\"; circuits = json.load(open(path)).get(\"circuits\", []) if os.path.exists(path) else []; sys.exit(0 if len(circuits) == $expected_count else 1)'"; }
dynamic_circuit_has_profile() { local lqos_ip=$1 expected_circuit=$2 expected_download=$3 expected_upload=$4; lab_ssh "$lqos_ip" "sudo python3 -c 'import json, math, os, sys; path = \"/opt/libreqos/src/dynamic_circuits.json\"; circuits = json.load(open(path)).get(\"circuits\", []) if os.path.exists(path) else []; matches = [c.get(\"shaped\", {}) for c in circuits if (\"$expected_circuit\" == \"any\" or c.get(\"shaped\", {}).get(\"circuit_id\") == \"$expected_circuit\")]; sys.exit(0 if any(math.isclose(float(c.get(\"download_max_mbps\", -1)), $expected_download) and math.isclose(float(c.get(\"upload_max_mbps\", -1)), $expected_upload) for c in matches) else 1)'"; }

wait_for_dynamic_circuit_profile() {
    local lqos_ip=$1 expected_circuit=$2 expected_download=$3 expected_upload=$4 description=$5
    for _ in $(seq 1 20); do dynamic_circuit_has_profile "$lqos_ip" "$expected_circuit" "$expected_download" "$expected_upload" && return 0; sleep 2; done
    die "$description"
}

run_pppoe_case() {
    local name=$1 username=$2 password=$3 expected_circuit=$4 expected_download=$5 expected_upload=$6
    note "checking that no dynamic circuit is present before $name"
    dynamic_circuit_count_is "$lqos_ip" 0 || die "a dynamic circuit was already present before $name"
    start_cursor=$(journal_cursor "$lqos_ip"); [[ -n $start_cursor ]] || die "could not establish an lqosd journal cursor"
    note "starting PPPoE session for $name"
    printf '%s\n' "plugin rp-pppoe.so pppoe0" "user $username" "password $password" noauth persist 'maxfail 1' | lab_ssh "$client_ip" 'sudo tee /etc/ppp/peers/radius-lab >/dev/null'
    lab_ssh "$client_ip" 'sudo poff radius-lab || true; sudo pon radius-lab'
    for _ in $(seq 1 30); do lab_ssh "$client_ip" "ip -4 addr show ppp0 | grep -q '100.64.0.2'" && break; sleep 2; done
    lab_ssh "$client_ip" "ip -4 addr show ppp0 | grep -q '100.64.0.2'" || die "PPPoE client did not receive 100.64.0.2 for $name"
    note "waiting for RADIUS Start and the expected $name queue"
    wait_for_log "$lqos_ip" "$start_cursor" 'accepted RADIUS accounting packet.*status=Some[(]Start[)]' "lqosd did not accept a RADIUS Start packet for $name"
    wait_for_log "$lqos_ip" "$start_cursor" 'applied RADIUS dynamic-circuit request.*intent="create"' "dynamic circuit creation was not observed for $name"
    wait_for_dynamic_circuit_profile "$lqos_ip" "$expected_circuit" "$expected_download" "$expected_upload" "the $name queue did not have the expected ${expected_download}/${expected_upload} Mbps profile"
    interim_cursor=$(journal_cursor "$lqos_ip"); note "waiting for the RouterOS five-second RADIUS Interim-Update for $name"
    wait_for_log "$lqos_ip" "$interim_cursor" 'accepted RADIUS accounting packet.*status=Some[(]InterimUpdate[)]' "lqosd did not accept a RADIUS Interim-Update packet for $name"
    wait_for_dynamic_circuit_profile "$lqos_ip" "$expected_circuit" "$expected_download" "$expected_upload" "the $name queue did not retain its expected profile after Interim-Update"
    stop_cursor=$(journal_cursor "$lqos_ip"); note "disconnecting $name and waiting for RADIUS Stop plus queue removal"
    lab_ssh "$client_ip" 'sudo poff radius-lab'
    wait_for_log "$lqos_ip" "$stop_cursor" 'accepted RADIUS accounting packet.*status=Some[(]Stop[)]' "lqosd did not accept a RADIUS Stop packet for $name"
    wait_for_log "$lqos_ip" "$stop_cursor" 'applied RADIUS dynamic-circuit request.*intent="remove"' "dynamic circuit removal was not observed for $name"
    for _ in $(seq 1 20); do dynamic_circuit_count_is "$lqos_ip" 0 && return 0; sleep 2; done
    die "the $name dynamic circuit remained after RADIUS Stop"
}

client_ip=$(wait_for_management_ip 52:54:00:30:00:10)
lqos_ip=$(wait_for_management_ip 52:54:00:10:00:10)
trap 'lab_ssh "$client_ip" "sudo poff radius-lab || true" >/dev/null 2>&1 || true' EXIT
run_pppoe_case 'packet-rate fallback identity' pppoe-rate pppoe-rate-password any 10 25
run_pppoe_case 'known username ShapedDevices identity' pppoe-known pppoe-known-password radius-known-circuit 60 20
run_pppoe_case 'unknown username default identity' pppoe-unknown pppoe-unknown-password any 30 12
trap - EXIT
note "all RADIUS lifecycle and queue-profile tests passed"
