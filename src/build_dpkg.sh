#!/bin/bash
set -euo pipefail

####################################################
# Copyright (c) 2022, Herbert Wolverson and LibreQoE
# This is all GPL2.

BUILD_DATE=$(date +%Y%m%d%H%M)
[ "${1:-}" = "--nostamp" ] && BUILD_DATE=""

PACKAGE=libreqos
VERSION=$(cat ./VERSION_STRING).$BUILD_DATE
PKGVERSION="${PACKAGE}_${VERSION}"
DPKG_DIR=dist/$PKGVERSION-1_amd64
APT_DEPENDENCIES="python3-pip, nano, curl, ca-certificates"
DEBIAN_DIR=$DPKG_DIR/DEBIAN
LQOS_DIR=$DPKG_DIR/opt/libreqos/src
LQOS_STATE_DIR=$DPKG_DIR/opt/libreqos/state
ETC_DIR=$DPKG_DIR/etc
ETC_LIBREQOS_DIR=$DPKG_DIR/etc/libreqos
MOTD_DIR=$DPKG_DIR/etc/update-motd.d
LQOS_FILES=(
  csvToNetworkJSON.py
  configMigrator.py
  integrationCommon.py
  integrationPowercode.py
  integrationNetzur.py
  integrationRestHttp.py
  integrationSonar.py
  integrationSplynx.py
  integrationVISP.py
  integrationWISPGate.py
  LibreQoS.py
  lqos.example
  lqTools.py
  mikrotikFindIPv6.py
  mikrotik_ipv6.example.toml
  network.example.json
  pythonCheck.py
  qoo_profiles.json
  scheduler.py
  ShapedDevices.example.csv
  shaping_skip_report.py
  systemd_hotfix.sh
  install_caddy.sh
  disable_caddy.sh
  virtual_tree_nodes.py
  manualNetwork.template.csv
  ../requirements.txt
  update_api.sh
)

LQOS_BIN_FILES=(
  lqos_scheduler.service.example
  lqosd.service.example
  lqos_api.service.example
  lqos_setup.service.example
)

RUSTPROGS=(
  lqosd
  lqos_netplan_helper
  lqtop
  xdp_iphash_to_cpu_cmdline
  xdp_pping
  lqusers
  lqos_setup
  lqos_map_perf
  uisp_integration
  lqos_overrides
  lqos_topology
)

####################################################
# Clean any previous dist build
rm -rf dist

####################################################
# Bump the build number

####################################################
# The Debian Packaging Bit

# Create the basic directory structure
mkdir -p "$LQOS_DIR"/bin/static2 "$DEBIAN_DIR" "$ETC_DIR" "$ETC_LIBREQOS_DIR" "$LQOS_DIR"/rust "$LQOS_DIR"/bin/dashboards
mkdir -p "$LQOS_STATE_DIR"/topology "$LQOS_STATE_DIR"/shaping "$LQOS_STATE_DIR"/stats "$LQOS_STATE_DIR"/cache "$LQOS_STATE_DIR"/debug "$LQOS_STATE_DIR"/quarantine

# shellcheck disable=SC2086
mkdir -p $MOTD_DIR

# Create the Debian control file
pushd "$DEBIAN_DIR" > /dev/null || exit
cat <<EOF > control
Package: $PACKAGE
Version: $VERSION
Architecture: amd64
Maintainer: Herbert Wolverson <herberticus@gmail.com>
Description: CAKE-based traffic shaping for ISPs
Depends: $APT_DEPENDENCIES
EOF
popd > /dev/null || exit

# Build the Rust programs (before the control file, we need to LDD lqosd).
# Keep package artifacts on the full release profile; build_rust.sh --fast is
# intentionally a local-iteration-only shortcut.
pushd rust > /dev/null || exit
# Build only required binaries and artifacts (exclude lqos_support_tool executable)
cargo build --release \
  -p lqosd \
  -p lqos_netplan_helper \
  -p lqtop \
  -p xdp_iphash_to_cpu_cmdline \
  -p xdp_pping \
  -p lqusers \
  -p lqos_setup \
  -p lqos_map_perf \
  -p uisp_integration \
  -p lqos_python \
  -p lqos_overrides \
  -p lqos_topology
popd > /dev/null || exit

# Create the post-installation file
pushd "$DEBIAN_DIR" > /dev/null || exit
cat <<'EOF' > postinst
#!/bin/bash
set -euo pipefail

set_libreqos_operator_permissions() {
local runtime_paths=()
[ -e /opt/libreqos/src ] && runtime_paths+=("/opt/libreqos/src")
[ -e /opt/libreqos/state ] && runtime_paths+=("/opt/libreqos/state")
[ ${#runtime_paths[@]} -eq 0 ] && return

if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ] && id "$SUDO_USER" >/dev/null 2>&1; then
    chown -R "$SUDO_USER:$SUDO_USER" "${runtime_paths[@]}"
    chmod -R u+rwX "${runtime_paths[@]}" || true
    echo "Granted $SUDO_USER ownership of /opt/libreqos/src and /opt/libreqos/state for SFTP editing."
else
    echo "Unable to determine the installing operator account automatically."
    echo "If you plan to edit LibreQoS files over SFTP, run: sudo chown -R <username>:<username> /opt/libreqos/src /opt/libreqos/state"
fi
}

# Install Python Dependencies
pushd /opt/libreqos > /dev/null
# - Setup Python dependencies as a post-install task. Use --ignore-installed so
#   pip does not try to uninstall Debian-managed packages that do not ship a
#   pip RECORD file (for example blinker on Ubuntu 24.04).
if [ -s src/deb-requirements-constraints.txt ]; then
PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install --ignore-installed -c src/deb-requirements-constraints.txt -r src/requirements.txt
else
PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install --ignore-installed -r src/requirements.txt
fi

# Ensure folder permissions are correct post-install
set_libreqos_operator_permissions

# - Setup the services
install -m 0644 /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
install -m 0644 /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
install -m 0644 /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
install -m 0644 /opt/libreqos/src/bin/lqos_setup.service.example /etc/systemd/system/lqos_setup.service
/bin/rm -f /etc/systemd/system/lqos_netplan_helper.service || true
/bin/systemctl daemon-reload || true
/bin/systemctl stop lqos_node_manager || true # In case it's running from a previous release
/bin/systemctl disable lqos_node_manager || true # In case it's running from a previous release
/bin/systemctl stop lqos_netplan_helper || true
/bin/systemctl disable lqos_netplan_helper || true
case "$(/opt/libreqos/src/bin/lqos_setup postinst-action)" in
activate_runtime)
  /opt/libreqos/src/bin/lqos_setup activate-runtime
  ;;
activate_setup)
  /opt/libreqos/src/bin/lqos_setup activate-setup
  /opt/libreqos/src/bin/lqos_setup print-link || true
  ;;
block_for_hotfix)
  /opt/libreqos/src/bin/lqos_setup hotfix-status || true
  cat <<'HOTFIX'

Install the Noble systemd hotfix before package configuration can start LibreQoS services.

Run:
  sudo /opt/libreqos/src/systemd_hotfix.sh install

Then re-run the LibreQoS package installation.
HOTFIX
  exit 1
  ;;
*)
  echo "Unknown LibreQoS post-install action." >&2
  exit 1
  ;;
esac
popd > /dev/null
EOF

# Uninstall Script
cat <<EOF > postrm
#!/bin/bash
set +e
/bin/systemctl stop lqos_netplan_helper lqosd lqos_scheduler lqos_api lqos_setup || true
/bin/systemctl disable lqos_netplan_helper lqosd lqos_scheduler lqos_api lqos_setup || true
/bin/rm -f /etc/systemd/system/lqos_netplan_helper.service || true
/bin/systemctl daemon-reload || true
exit 0
EOF
chmod a+x postinst postrm
popd > /dev/null || exit

# Copy files into the LibreQoS directory
for file in "${LQOS_FILES[@]}"; do
  if [ ! -f "$file" ]; then
    echo "Missing packaged file: $file" >&2
    exit 1
  fi
done

for file in "${LQOS_FILES[@]}"; do
  cp "$file" "$LQOS_DIR"
done

if [ -f deb-requirements-constraints.txt ]; then
  cp deb-requirements-constraints.txt "$LQOS_DIR"
fi

cp mikrotik_ipv6.example.toml "$ETC_LIBREQOS_DIR"/mikrotik_ipv6.example.toml

# Ensure helper scripts are executable in the package
for helper_script in update_api.sh install_caddy.sh disable_caddy.sh; do
  if [ -f "$LQOS_DIR/$helper_script" ]; then
    chmod a+x "$LQOS_DIR/$helper_script"
  fi
done

# Copy files into the LibreQoS/bin directory
for file in "${LQOS_BIN_FILES[@]}"; do
  cp "bin/$file" "$LQOS_DIR/bin"
done

# Copy the remove pinned maps
cp rust/remove_pinned_maps.sh "$LQOS_DIR"/rust

####################################################

# Compile the website
pushd rust/lqosd > /dev/null || exit
./copy_files.sh
popd || exit

# Copy newly built Rust files
# - The Python integration Library
cp rust/target/release/liblqos_python.so "$LQOS_DIR"
# - The main executables
for prog in "${RUSTPROGS[@]}"; do
  cp rust/target/release/"$prog" "$LQOS_DIR"/bin
done

cp -r bin/static2/* "$LQOS_DIR"/bin/static2

cat <<EOF > "$LQOS_DIR/bin/dashboards/default.json"
{"name":"default","entries":[
  {"name":"Shaped/Unshaped Pie","tag":"shapedUnshaped","size":2},
  {"name":"Last 5 Minutes Throughput","tag":"throughputRing","size":4},
  {"name":"Total TCP Retransmits","tag":"totalRetransmits","size":4},
  {"name":"RAM Utilization","tag":"ram","size":2},
  {"name":"Throughput Packets/Second","tag":"throughputPps","size":2},
  {"name":"Round-Trip Time Histogram","tag":"rttHistogram","size":2},
  {"name":"Total Cake Stats","tag":"totalCakeStats","size":4},
  {"name":"Tracked Flows Counter","tag":"trackedFlowsCount","size":2},
  {"name":"CPU Utilization","tag":"cpu","size":2},
  {"name":"Network Tree Sankey","tag":"networkTreeSankey","size":6},
  {"name":"Network Tree Summary","tag":"treeSummary","size":6},
  {"name":"Top 10 Downloaders (Visual)","tag":"top10downloadersV","size":6},
  {"name":"Top 10 Downloaders","tag":"top10downloaders","size":6},
  {"name":"Worst 10 Round-Trip Time (Visual)","tag":"worst10downloadersV","size":6},
  {"name":"Worst 10 Round-Trip Time","tag":"worst10downloaders","size":6},
  {"name":"Worst 10 Retransmits (Visual)","tag":"worst10retransmitsV","size":6},
  {"name":"Worst 10 Retransmits","tag":"worst10retransmits","size":6},
  {"name":"Top 10 Flows (total bytes)","tag":"top10flowsBytes","size":6},
  {"name":"Top 10 Flows (rate)","tag":"top10flowsRate","size":6},
  {"name":"Top 10 Endpoints by Country","tag":"top10endpointsCountry","size":6},
  {"name":"Ether Protocols","tag":"etherProtocols","size":6},
  {"name":"IP Protocols","tag":"ipProtocols","size":6},
  {"name":"Combined Top 10 Box","tag":"combinedTop10","size":6},
  {"name":"Circuits At Capacity","tag":"circuitCapacity","size":6},
  {"name":"Tree Nodes At Capacity","tag":"treeCapacity","size":6}
]}
EOF

####################################################
# Add Message of the Day
pushd "$MOTD_DIR" > /dev/null || exit
cat <<EOF > 99-libreqos
#!/bin/bash
MY_IP=$(hostname -I | cut -d' ' -f1)
echo \"\"
echo "LibreQoS Traffic Shaper is installed on this machine."
echo "Point a browser at http://\$MY_IP:9123/ to manage it."
echo \"\"
EOF
popd || exit

####################################################
# Bundle the API into the package
echo "Fetching lqos_api via update_api.sh ..."
bash ./update_api.sh --bin-dir "$LQOS_DIR/bin" --no-restart
if [[ ! -x "$LQOS_DIR/bin/lqos_api" ]]; then
  echo "Error: lqos_api was not installed into the package at $LQOS_DIR/bin/lqos_api"
  exit 1
fi

####################################################
# Assemble the package
dpkg-deb --root-owner-group --build "$DPKG_DIR"
