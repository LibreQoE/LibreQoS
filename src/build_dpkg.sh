#!/bin/bash

####################################################
# Copyright (c) 2022, Herbert Wolverson and LibreQoE
# This is all GPL2.

BUILD_DATE=$(date +%Y%m%d%H%M)
if [ "$1" = "--nostamp" ]
then
    BUILD_DATE=""
fi
PACKAGE=libreqos
VERSION=$(cat ./VERSION_STRING).$BUILD_DATE
PKGVERSION=$PACKAGE
PKGVERSION+="_"
PKGVERSION+=$VERSION
DPKG_DIR=dist/$PKGVERSION-1_amd64
APT_DEPENDENCIES="python3-pip, nano, graphviz, curl"
DEBIAN_DIR=$DPKG_DIR/DEBIAN
LQOS_DIR=$DPKG_DIR/opt/libreqos/src
ETC_DIR=$DPKG_DIR/etc
MOTD_DIR=$DPKG_DIR/etc/update-motd.d
LQOS_FILES="csvToNetworkJSON.py integrationCommon.py integrationPowercode.py integrationRestHttp.py integrationSonar.py integrationSplynx.py integrationUISP.py integrationSonar.py LibreQoS.py lqos.example lqTools.py mikrotikFindIPv6.py network.example.json pythonCheck.py README.md scheduler.py ShapedDevices.example.csv mikrotikDHCPRouterList.csv integrationUISPbandwidths.template.csv manualNetwork.template.csv integrationUISProutes.template.csv integrationSplynxBandwidths.template.csv lqos.example ../requirements.txt"
LQOS_BIN_FILES="lqos_scheduler.service.example lqosd.service.example"
RUSTPROGS="lqosd lqtop xdp_iphash_to_cpu_cmdline xdp_pping lqusers lqos_setup lqos_map_perf uisp_integration lqos_support_tool"

####################################################
# Clean any previous dist build
rm -rf dist

####################################################
# Bump the build number

####################################################
# The Debian Packaging Bit

# Create the basic directory structure
mkdir -p "$DEBIAN_DIR"

# Build the chroot directory structure
mkdir -p "$LQOS_DIR"
mkdir -p "$LQOS_DIR"/bin/static2
mkdir -p "$ETC_DIR"
# shellcheck disable=SC2086
mkdir -p $MOTD_DIR

# Create the Debian control file
pushd "$DEBIAN_DIR" > /dev/null || exit
touch control
echo "Package: $PACKAGE" >> control
echo "Version: $VERSION" >> control
echo "Architecture: amd64" >> control
echo "Maintainer: Herbert Wolverson <herberticus@gmail.com>" >> control
echo "Description: CAKE-based traffic shaping for ISPs" >> control
echo "Depends: $APT_DEPENDENCIES" >> control
popd > /dev/null || exit

# Build the Rust programs (before the control file, we need to LDD lqosd)
pushd rust > /dev/null || exit
#cargo clean
cargo build --all --release
popd > /dev/null || exit
LINKED_PYTHON=$(ldd rust/target/release/lqosd | grep libpython | sed -e '/^[^\t]/ d' | sed -e 's/\t//' | sed -e 's/.*=..//' | sed -e 's/ (0.*)//')

# Create the post-installation file
pushd "$DEBIAN_DIR" > /dev/null || exit
touch postinst
echo "#!/bin/bash" >> postinst
echo "# Install Python Dependencies" >> postinst
echo "pushd /opt/libreqos" >> postinst
# - Setup Python dependencies as a post-install task
echo "PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install -r src/requirements.txt" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 python3 -m pip install -r src/requirements.txt" >> postinst
# - Setup Python dependencies as a post-install task - handle issue with two packages on Ubuntu Server 24.04
echo "PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall binpacking --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall binpacking --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install binpacking" >> postinst
echo "PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall apscheduler --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall apscheduler --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install apscheduler" >> postinst
echo "PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall deepdiff --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip uninstall deepdiff --yes" >> postinst
echo "sudo PIP_BREAK_SYSTEM_PACKAGES=1 pip install deepdiff" >> postinst
# Ensure folder permissions are correct post-install
echo "sudo chown -R $USER /opt/libreqos" >> postinst
# - Run lqsetup
echo "/opt/libreqos/src/bin/lqos_setup" >> postinst
# - Setup the services
echo "cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service" >> postinst
echo "cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service" >> postinst
echo "/bin/systemctl daemon-reload" >> postinst
echo "/bin/systemctl stop lqos_node_manager" >> postinst # In case it's running from a previous release
echo "/bin/systemctl disable lqos_node_manager" >> postinst # In case it's running from a previous release
echo "/bin/systemctl enable lqosd lqos_scheduler" >> postinst
echo "/bin/systemctl start lqosd" >> postinst
echo "/bin/systemctl start lqos_scheduler" >> postinst
echo "popd" >> postinst
# Attempting to fixup versioning issues with libpython.
# This requires that you already have LibreQoS installed.
echo "if ! test -f $LINKED_PYTHON; then" >> postinst
echo "  if test -f /lib/x86_64-linux-gnu/libpython3.12.so.1.0; then" >> postinst
echo "    ln -s /lib/x86_64-linux-gnu/libpython3.12.so.1.0 $LINKED_PYTHON" >> postinst
echo "  fi" >> postinst
echo "  if test -f /lib/x86_64-linux-gnu/libpython3.11.so.1.0; then" >> postinst
echo "    ln -s /lib/x86_64-linux-gnu/libpython3.11.so.1.0 $LINKED_PYTHON" >> postinst
echo "  fi" >> postinst
echo "fi" >> postinst
# End of symlink insanity
chmod a+x postinst

# Uninstall Script
touch postrm
echo "#!/bin/bash" >> postrm
echo "/bin/systemctl stop lqosd" >> postrm
echo "/bin/systemctl stop lqos_scheduler" >> postrm
echo "/bin/systemctl disable lqosd lqos_scheduler" >> postrm
chmod a+x postrm
popd > /dev/null || exit

# Create the cleanup file
pushd "$DEBIAN_DIR" > /dev/null || exit
touch postrm
echo "#!/bin/bash" >> postrm
chmod a+x postrm
popd > /dev/null || exit

# Copy files into the LibreQoS directory
for file in $LQOS_FILES
do
    cp "$file" "$LQOS_DIR"
done

# Copy files into the LibreQoS/bin directory
for file in $LQOS_BIN_FILES
do
    cp bin/"$file" "$LQOS_DIR"/bin
done

# Copy the remove pinned maps
mkdir -p "$LQOS_DIR"/rust
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
for prog in $RUSTPROGS
do
    cp rust/target/release/"$prog" "$LQOS_DIR"/bin
done

cp -r bin/static2/* "$LQOS_DIR"/bin/static2
mkdir "$LQOS_DIR"/bin/dashboards
echo "[{\"name\":\"Throughput Bits/Second\",\"tag\":\"throughputBps\",\"size\":2},{\"name\":\"Shaped/Unshaped Pie\",\"tag\":\"shapedUnshaped\",\"size\":2},{\"name\":\"Throughput Packets/Second\",\"tag\":\"throughputPps\",\"size\":2},{\"name\":\"Tracked Flows Counter\",\"tag\":\"trackedFlowsCount\",\"size\":2},{\"name\":\"Last 5 Minutes Throughput\",\"tag\":\"throughputRing\",\"size\":2},{\"name\":\"Round-Trip Time Histogram\",\"tag\":\"rttHistogram\",\"size\":2},{\"name\":\"Network Tree Sankey\",\"tag\":\"networkTreeSankey\",\"size\":6},{\"name\":\"Network Tree Summary\",\"tag\":\"treeSummary\",\"size\":6},{\"name\":\"Top 10 Downloaders\",\"tag\":\"top10downloaders\",\"size\":6},{\"name\":\"Worst 10 Round-Trip Time\",\"tag\":\"worst10downloaders\",\"size\":6},{\"name\":\"Worst 10 Retransmits\",\"tag\":\"worst10retransmits\",\"size\":6},{\"name\":\"Top 10 Flows (total bytes)\",\"tag\":\"top10flowsBytes\",\"size\":6},{\"name\":\"Top 10 Flows (rate)\",\"tag\":\"top10flowsRate\",\"size\":6},{\"name\":\"Top 10 Endpoints by Country\",\"tag\":\"top10endpointsCountry\",\"size\":6},{\"name\":\"Ether Protocols\",\"tag\":\"etherProtocols\",\"size\":6},{\"name\":\"IP Protocols\",\"tag\":\"ipProtocols\",\"size\":6},{\"name\":\"CPU Utilization\",\"tag\":\"cpu\",\"size\":3},{\"name\":\"RAM Utilization\",\"tag\":\"ram\",\"size\":3},{\"name\":\"Combined Top 10 Box\",\"tag\":\"combinedTop10\",\"size\":6},{\"name\":\"Total Cake Stats\",\"tag\":\"totalCakeStats\",\"size\":6},{\"name\":\"Circuits At Capacity\",\"tag\":\"circuitCapacity\",\"size\":6},{\"name\":\"Tree Nodes At Capacity\",\"tag\":\"treeCapacity\",\"size\":6}]" > "$LQOS_DIR"/bin/dashboards/default.json

####################################################
# Add Message of the Day
pushd "$MOTD_DIR" > /dev/null || exit
echo "#!/bin/bash" > 99-libreqos
printf "MY_IP=\'hostname -I | cut -d' ' -f1\'" >> 99-libreqos
echo "echo \"\"" >> 99-libreqos
echo "echo \"LibreQoS Traffic Shaper is installed on this machine.\"" >> 99-libreqos
echo "echo \"Point a browser at http://\$MY_IP:9123/ to manage it.\"" >> 99-libreqos
echo "echo \"\"" >> 99-libreqos
chmod a+x 99-libreqos
popd || exit

####################################################
# Assemble the package
dpkg-deb --root-owner-group --build "$DPKG_DIR"
