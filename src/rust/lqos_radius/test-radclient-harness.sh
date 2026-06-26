#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: test-radclient-harness.sh

Builds the lqos_radius diagnostic listener, starts it on loopback, and sends
RADIUS Accounting-Request packets with radclient.

Environment overrides:
  RADIUS_HARNESS_PORT=18130
  RADIUS_HARNESS_SECRET=testing123
  RADIUS_HARNESS_CLIENT_SOURCE=127.0.0.1
  RADIUS_HARNESS_TIMEOUT=2
  RADIUS_HARNESS_READY_ATTEMPTS=10
  KEEP_RADIUS_HARNESS_TMP=1
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
PORT="${RADIUS_HARNESS_PORT:-18130}"
SECRET="${RADIUS_HARNESS_SECRET:-testing123}"
CLIENT_SOURCE="${RADIUS_HARNESS_CLIENT_SOURCE:-127.0.0.1}"
RADCLIENT_TIMEOUT="${RADIUS_HARNESS_TIMEOUT:-2}"
READY_ATTEMPTS="${RADIUS_HARNESS_READY_ATTEMPTS:-10}"
LISTEN_ADDR="127.0.0.1:${PORT}"

if ! command -v radclient >/dev/null 2>&1; then
    echo "radclient is required but was not found in PATH" >&2
    exit 1
fi

case "${PORT}" in
    ''|*[!0-9]*)
        echo "RADIUS_HARNESS_PORT must be numeric, got '${PORT}'" >&2
        exit 1
        ;;
esac
if (( PORT < 1024 || PORT > 65535 )); then
    echo "RADIUS_HARNESS_PORT must be in the unprivileged range 1024..65535, got '${PORT}'" >&2
    exit 1
fi
case "${READY_ATTEMPTS}" in
    ''|*[!0-9]*)
        echo "RADIUS_HARNESS_READY_ATTEMPTS must be numeric, got '${READY_ATTEMPTS}'" >&2
        exit 1
        ;;
esac
if (( READY_ATTEMPTS < 1 )); then
    echo "RADIUS_HARNESS_READY_ATTEMPTS must be greater than zero" >&2
    exit 1
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/lqos-radius-radclient.XXXXXX")"
LISTENER_LOG="${WORK_DIR}/listener.log"
LISTENER_PID=""

cleanup() {
    if [[ -n "${LISTENER_PID}" ]] && kill -0 "${LISTENER_PID}" >/dev/null 2>&1; then
        kill "${LISTENER_PID}" >/dev/null 2>&1 || true
        wait "${LISTENER_PID}" >/dev/null 2>&1 || true
    fi

    if [[ "${KEEP_RADIUS_HARNESS_TMP:-0}" == "1" ]]; then
        echo "Kept harness files in ${WORK_DIR}"
    else
        rm -rf "${WORK_DIR}"
    fi
}
trap cleanup EXIT

require_listener_log() {
    if [[ -s "${LISTENER_LOG}" ]]; then
        cat "${LISTENER_LOG}" >&2
    else
        echo "listener produced no log output" >&2
    fi
}

wait_for_listener() {
    local output_file="${WORK_DIR}/ready.radclient.out"

    for _ in $(seq 1 "${READY_ATTEMPTS}"); do
        if radclient -x -r 1 -t "${RADCLIENT_TIMEOUT}" "${LISTEN_ADDR}" acct "${SECRET}" \
            <"${WORK_DIR}/acct-probe.rad" >"${output_file}" 2>&1
        then
            return 0
        fi
        if ! kill -0 "${LISTENER_PID}" >/dev/null 2>&1; then
            echo "lqos_radius listener exited before becoming ready" >&2
            cat "${output_file}" >&2
            require_listener_log
            exit 1
        fi
        sleep 0.1
    done

    echo "timed out waiting for lqos_radius listener on ${LISTEN_ADDR}" >&2
    cat "${output_file}" >&2
    require_listener_log
    exit 1
}

write_common_session_attributes() {
    local framed_ip="$1"

    cat <<EOF
User-Name = "customer-123"
Acct-Session-Id = "sess-0001"
NAS-IP-Address = 192.0.2.10
NAS-Identifier = "pppoe-core-1"
NAS-Port = 501
NAS-Port-Type = Virtual
Calling-Station-Id = "AA-BB-CC-DD-EE-FF"
Called-Station-Id = "pppoe-access"
Service-Type = Framed-User
Framed-Protocol = PPP
Framed-IP-Address = ${framed_ip}
EOF
}

write_accounting_fixture() {
    local path="$1"
    local status="$2"
    local framed_ip="$3"
    shift 3

    {
        echo "Acct-Status-Type = ${status}"
        write_common_session_attributes "${framed_ip}"
        for attribute in "$@"; do
            printf '%s\n' "${attribute}"
        done
    } >"${path}"
}

write_radius_fixtures() {
    cat >"${WORK_DIR}/acct-probe.rad" <<'EOF'
Acct-Status-Type = Accounting-On
NAS-IP-Address = 192.0.2.10
NAS-Identifier = "pppoe-core-1"
EOF

    write_accounting_fixture \
        "${WORK_DIR}/acct-start.rad" \
        "Start" \
        "198.51.100.55"

    write_accounting_fixture \
        "${WORK_DIR}/acct-interim.rad" \
        "Interim-Update" \
        "198.51.100.56" \
        "Acct-Input-Octets = 123456" \
        "Acct-Output-Octets = 654321" \
        "Acct-Session-Time = 60"

    write_accounting_fixture \
        "${WORK_DIR}/acct-stop.rad" \
        "Stop" \
        "198.51.100.56" \
        "Acct-Input-Octets = 223456" \
        "Acct-Output-Octets = 754321" \
        "Acct-Session-Time = 120" \
        "Acct-Terminate-Cause = User-Request"
}

send_accounting_packet() {
    local label="$1"
    local packet_file="$2"
    local output_file="${WORK_DIR}/${label}.radclient.out"

    if ! radclient -x -r 1 -t "${RADCLIENT_TIMEOUT}" "${LISTEN_ADDR}" acct "${SECRET}" \
        <"${packet_file}" >"${output_file}" 2>&1
    then
        echo "radclient ${label} request failed" >&2
        cat "${output_file}" >&2
        require_listener_log
        exit 1
    fi
}

send_rejected_packet() {
    local output_file="${WORK_DIR}/wrong-secret.radclient.out"

    if radclient -x -r 1 -t "${RADCLIENT_TIMEOUT}" "${LISTEN_ADDR}" acct "wrong-${SECRET}" \
        <"${WORK_DIR}/acct-start.rad" >"${output_file}" 2>&1
    then
        echo "radclient unexpectedly received a response for a wrong-secret request" >&2
        cat "${output_file}" >&2
        require_listener_log
        exit 1
    fi
}

echo "Building lqos_radius diagnostic listener..."
cargo build --manifest-path "${RUST_DIR}/Cargo.toml" -p lqos_radius

write_radius_fixtures

"${RUST_DIR}/target/debug/lqos_radius" \
    --listen "${LISTEN_ADDR}" \
    --client-source "${CLIENT_SOURCE}" \
    --shared-secret "${SECRET}" \
    >"${LISTENER_LOG}" 2>&1 &
LISTENER_PID="$!"

wait_for_listener

echo "Sending accepted Accounting-Start, Interim-Update, and Stop packets..."
send_accounting_packet "start" "${WORK_DIR}/acct-start.rad"
send_accounting_packet "interim" "${WORK_DIR}/acct-interim.rad"
send_accounting_packet "stop" "${WORK_DIR}/acct-stop.rad"

echo "Sending wrong-secret rejection probe..."
send_rejected_packet

echo "RADIUS radclient loopback harness passed on ${LISTEN_ADDR}"
