#!/bin/bash

####################################################
# Copyright (c) 2022, Herbert Wolverson and LibreQoE
# This is all GPL2.

BUILD_DATE=$(date +%Y%m%d%H%M)
[ "$1" = "--nostamp" ] && BUILD_DATE=""

PACKAGE=libreqos
VERSION=$(cat ./VERSION_STRING).$BUILD_DATE
PKGVERSION="${PACKAGE}_${VERSION}"
DPKG_DIR=dist/$PKGVERSION-1_amd64
APT_DEPENDENCIES="python3-pip, nano, graphviz, curl, ca-certificates"
DEBIAN_DIR=$DPKG_DIR/DEBIAN
LQOS_DIR=$DPKG_DIR/opt/libreqos/src
ETC_DIR=$DPKG_DIR/etc
MOTD_DIR=$DPKG_DIR/etc/update-motd.d
LQOS_FILES=(
  csvToNetworkJSON.py
  integrationCommon.py
  integrationPowercode.py
  integrationNetzur.py
  integrationRestHttp.py
  integrationSonar.py
  integrationSplynx.py
  integrationUISP.py
  LibreQoS.py
  lqos.example
  lqTools.py
  mikrotikFindIPv6.py
  network.example.json
  pythonCheck.py
  README.md
  scheduler.py
  ShapedDevices.example.csv
  mikrotikDHCPRouterList.template.csv
  integrationUISPbandwidths.template.csv
  manualNetwork.template.csv
  integrationUISProutes.template.csv
  integrationSplynxBandwidths.template.csv
  ../requirements.txt
  update_api.sh
)

LQOS_BIN_FILES=(
  lqos_scheduler.service.example
  lqosd.service.example
  lqos_api.service.example
)

RUSTPROGS=(
  lqosd
  lqtop
  xdp_iphash_to_cpu_cmdline
  xdp_pping
  lqusers
  lqos_setup
  lqos_map_perf
  uisp_integration
  lqos_support_tool
  lqos_overrides
)

####################################################
# Clean any previous dist build
rm -rf dist

####################################################
# Bump the build number

####################################################
# The Debian Packaging Bit

# Create the basic directory structure
mkdir -p "$LQOS_DIR"/bin/static2 "$DEBIAN_DIR" "$ETC_DIR" "$LQOS_DIR"/rust "$LQOS_DIR"/bin/dashboards

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

# Build the Rust programs (before the control file, we need to LDD lqosd)
pushd rust > /dev/null || exit
# Build only required binaries and artifacts (exclude lqos_support_tool executable)
cargo build --release \
  -p lqosd \
  -p lqtop \
  -p xdp_iphash_to_cpu_cmdline \
  -p xdp_pping \
  -p lqusers \
  -p lqos_setup \
  -p lqos_map_perf \
  -p uisp_integration \
  -p lqos_python \
  -p lqos_overrides
popd > /dev/null || exit

# Create the post-installation file
pushd "$DEBIAN_DIR" > /dev/null || exit
cat <<EOF > postinst
#!/bin/bash
# Install Python Dependencies
pushd /opt/libreqos
# - Setup Python dependencies as a post-install task
PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install -r src/requirements.txt
sudo PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install -r src/requirements.txt
# - Setup Python dependencies as a post-install task - handle issue with packages on Ubuntu Server 24.04
sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall apscheduler deepdiff --yes
PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall apscheduler deepdiff --yes
sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install apscheduler deepdiff

# Ensure folder permissions are correct post-install
sudo chown -R $USER /opt/libreqos
# - Run lqsetup
/opt/libreqos/src/bin/lqos_setup
# - Setup the services
cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service
cp /opt/libreqos/src/bin/lqos_api.service.example /etc/systemd/system/lqos_api.service
/bin/systemctl daemon-reload || true
/bin/systemctl stop lqos_node_manager || true # In case it's running from a previous release
/bin/systemctl disable lqos_node_manager || true # In case it's running from a previous release
/bin/systemctl enable lqosd lqos_scheduler lqos_api || true
/bin/systemctl start lqosd lqos_scheduler lqos_api || true
EOF

# Uninstall Script
cat <<EOF > postrm
#!/bin/bash
set +e
/bin/systemctl stop lqosd lqos_scheduler lqos_api || true
/bin/systemctl disable lqosd lqos_scheduler lqos_api || true
/bin/systemctl daemon-reload || true
exit 0
EOF
chmod a+x postinst postrm
popd > /dev/null || exit

# Copy files into the LibreQoS directory
for file in "${LQOS_FILES[@]}"; do
  cp "$file" "$LQOS_DIR" || echo "Error copying $file"
done

# Ensure update_api.sh is executable in the package
if [ -f "$LQOS_DIR/update_api.sh" ]; then
  chmod a+x "$LQOS_DIR/update_api.sh" || true
fi

# Copy files into the LibreQoS/bin directory
for file in "${LQOS_BIN_FILES[@]}"; do
  cp "bin/$file" "$LQOS_DIR/bin" || echo "Error copying $file"
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
  cp rust/target/release/"$prog" "$LQOS_DIR"/bin || echo "Error copying $prog"
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
