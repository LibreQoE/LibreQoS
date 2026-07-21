#!/usr/bin/env bash
set -euo pipefail

LAB_ROOT=$(cd -- "$(dirname -- "$0")/.." && pwd)
source "$LAB_ROOT/lib/common.sh"
source "$LAB_ROOT/lab.env"

note "checking configuration prerequisites"
require_command sshpass
require_lab_secret RADIUS_SHARED_SECRET
require_lab_secret ROUTEROS_ADMIN_PASSWORD

note "discovering guest management addresses"
lqos_ip=$(wait_for_management_ip 52:54:00:10:00:10)
radius_ip=$(wait_for_management_ip 52:54:00:20:00:10)
client_ip=$(wait_for_management_ip 52:54:00:30:00:10)
router_ip=$(wait_for_management_ip 52:54:00:40:00:10)
note "waiting for SSH on LibreQoS, FreeRADIUS, and PPPoE client guests"
wait_for_ssh "$lqos_ip"; wait_for_ssh "$radius_ip"; wait_for_ssh "$client_ip"

note "rendering isolated lab configuration"
install -d -m 0700 "$RUN_DIR/config"
umask 077
cp "$LAB_ROOT/templates/lqos.conf" "$RUN_DIR/config/lqos.conf"
cp "$LAB_ROOT/templates/network.json" "$RUN_DIR/config/network.json"
cp "$LAB_ROOT/templates/ShapedDevices.csv" "$RUN_DIR/config/ShapedDevices.csv"
escaped_radius_secret=$(printf '%s' "$RADIUS_SHARED_SECRET" | sed 's/[\\/&]/\\&/g')
sed "s/__RADIUS_SHARED_SECRET__/$escaped_radius_secret/g" "$LAB_ROOT/templates/freeradius-client.conf" >"$RUN_DIR/config/radius-lab.conf"
sed "s/__RADIUS_SHARED_SECRET__/$escaped_radius_secret/g" "$LAB_ROOT/templates/routeros.rsc" >"$RUN_DIR/config/radius-lab.rsc"

note "uploading host-built LibreQoS runtime and fixtures"
lab_ssh "$lqos_ip" 'rm -rf /tmp/runtime'
lab_scp "$RUNTIME_DIR" "$lqos_ip" /tmp/
for file in lqos.conf network.json ShapedDevices.csv; do lab_scp "$RUN_DIR/config/$file" "$lqos_ip" "/tmp/$file"; done
lab_scp "$RUN_DIR/config/radius-lab.conf" "$radius_ip" /home/lab/radius-lab.conf
lab_scp "$LAB_ROOT/templates/freeradius-users" "$radius_ip" /tmp/users

note "installing LibreQoS fixture and RADIUS listener configuration"
lab_ssh "$lqos_ip" 'sudo mkdir -p /opt/libreqos/src /etc/libreqos; sudo rm -rf /opt/libreqos/src/*; sudo cp -a /tmp/runtime/src/. /opt/libreqos/src/; sudo mv /tmp/runtime/bin /opt/libreqos/src/bin; sudo mv /tmp/runtime/liblqos_python.so /opt/libreqos/src/liblqos_python.so; rm -rf /tmp/runtime; sudo install -m 0644 /tmp/lqos.conf /etc/lqos.conf; sudo install -m 0644 /tmp/network.json /opt/libreqos/src/network.json; sudo install -m 0644 /tmp/ShapedDevices.csv /opt/libreqos/src/ShapedDevices.csv'
note "creating isolated multiqueue veth interfaces for LibreQoS validation"
lab_ssh "$lqos_ip" 'for interface in lqos-lan lqos-wan; do sudo ip link del "$interface" 2>/dev/null || true; sudo ip link add "$interface" numrxqueues 2 numtxqueues 2 type veth peer name "${interface}-p" numrxqueues 2 numtxqueues 2; sudo ip link set "$interface" up; sudo ip link set "${interface}-p" up; done'
printf '%s\n' "$RADIUS_SHARED_SECRET" | lab_ssh "$lqos_ip" 'sudo tee /etc/libreqos/radius-shared-secret >/dev/null; sudo chmod 600 /etc/libreqos/radius-shared-secret'
lab_ssh "$lqos_ip" "sudo sysctl -w net.ipv4.ip_forward=1 >/dev/null; sudo nft delete table ip radius_lab 2>/dev/null || true; sudo nft add table ip radius_lab; sudo nft 'add chain ip radius_lab prerouting { type nat hook prerouting priority dstnat; }'; sudo nft 'add chain ip radius_lab postrouting { type nat hook postrouting priority srcnat; }'; sudo nft add rule ip radius_lab prerouting ip daddr 198.18.10.10 udp dport 1812 dnat to 198.18.10.20; sudo nft add rule ip radius_lab postrouting ip daddr 198.18.10.20 udp dport 1812 snat to 198.18.10.10"

note "configuring and starting FreeRADIUS"
lab_ssh "$radius_ip" "sudo install -m 0644 /tmp/users /etc/freeradius/3.0/mods-config/files/authorize; sudo grep -q 'client radius-lqos-nat' /etc/freeradius/3.0/clients.conf || sudo sh -c 'cat /home/lab/radius-lab.conf >> /etc/freeradius/3.0/clients.conf'; rm -f /home/lab/radius-lab.conf; sudo systemctl enable --now freeradius; sudo systemctl restart freeradius"
note "starting lqosd before building the LibreQoS fixture state"
lab_ssh "$lqos_ip" 'sudo systemctl stop radius-lqosd.service 2>/dev/null || true; sudo systemd-run --unit=radius-lqosd --collect --setenv=RUST_LOG=debug /opt/libreqos/src/bin/lqosd; sleep 1; sudo systemctl is-active --quiet radius-lqosd.service'
note "building the LibreQoS fixture state inside the lab"
lab_ssh "$lqos_ip" 'cd /opt/libreqos/src && sudo env PYTHONPATH=/opt/libreqos/src python3 LibreQoS.py'

note "uploading RouterOS PPPoE and RADIUS configuration"
export SSHPASS=$ROUTEROS_ADMIN_PASSWORD
sshpass -e scp -p -o UserKnownHostsFile="$RUN_DIR/routeros_known_hosts" -o StrictHostKeyChecking=accept-new "$RUN_DIR/config/radius-lab.rsc" "admin@$router_ip:radius-lab.rsc"
sshpass -e ssh -o UserKnownHostsFile="$RUN_DIR/routeros_known_hosts" -o StrictHostKeyChecking=accept-new "admin@$router_ip" '/import file-name=radius-lab.rsc'
unset SSHPASS

note "creating PPPoE client profile"
printf '%s\n' 'plugin rp-pppoe.so pppoe0' 'user pppoe-lab' 'password pppoe-lab-password' 'noauth' 'persist' 'maxfail 1' | lab_ssh "$client_ip" 'sudo tee /etc/ppp/peers/radius-lab >/dev/null'
note "guest configuration complete; run ./radius-harness/lab test"
