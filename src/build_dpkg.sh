#!/bin/bash

####################################################
# Copyright (c) 2022, Herbert Wolverson and LibreQoE
# This is all GPL2.

BUILD_DATE=`date +%Y%m%d%H%M`
if [[ "$1" == "--nostamp" ]]
then
    BUILD_DATE=""
fi
PACKAGE=libreqos
VERSION=`cat ./VERSION_STRING`.$BUILD_DATE
PKGVERSION=$PACKAGE
PKGVERSION+="_"
PKGVERSION+=$VERSION
DPKG_DIR=dist/$PKGVERSION-1_amd64
APT_DEPENDENCIES="python3-pip, nano, graphviz, curl"
DEBIAN_DIR=$DPKG_DIR/DEBIAN
LQOS_DIR=`pwd`/$DPKG_DIR/opt/libreqos/src
ETC_DIR=$DPKG_DIR/etc
MOTD_DIR=$DPKG_DIR/etc/update-motd.d
LQOS_FILES="graphInfluxDB.py influxDBdashboardTemplate.json integrationCommon.py integrationPowercode.py integrationRestHttp.py integrationSonar.py integrationSplynx.py integrationUISP.py LibreQoS.py lqos.example lqTools.py mikrotikFindIPv6.py network.example.json pythonCheck.py README.md scheduler.py ShapedDevices.example.csv ../requirements.txt"
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
# Build the chroot directory structure
mkdir -p $DEBIAN_DIR $LQOS_DIR/bin/static2 $ETC_DIR $MOTD_DIR

# Create the Debian control file
pushd $DEBIAN_DIR > /dev/null || exit
cat << EOF > control
Package: $PACKAGE
Version: $VERSION
Architecture: amd64
Maintainer: Herbert Wolverson <herberticus@gmail.com>
Description: CAKE-based traffic shaping for ISPs
Depends: $APT_DEPENDENCIES
EOF
popd > /dev/null || exit

# Create the post-installation file
pushd $DEBIAN_DIR > /dev/null || exit

cat << 'EOF' > postinst
#!/bin/bash
# Install Python Dependencies
pushd /opt/libreqos  > /dev/null || exit

# - Setup Python dependencies as a post-install task
python3 -m pip install --root-user-action=ignore --quiet --break-system-packages -r src/requirements.txt
sudo python3 -m pip install --root-user-action=ignore --quiet --break-system-packages -r src/requirements.txt

# - Run lqsetup
/opt/libreqos/src/bin/lqos_setup

# - Setup the services
cp /opt/libreqos/src/bin/lqosd.service.example /etc/systemd/system/lqosd.service
cp /opt/libreqos/src/bin/lqos_scheduler.service.example /etc/systemd/system/lqos_scheduler.service

service_exists() {
    local n=$1
    if [[ $(systemctl list-units --all -t service --full --no-legend "$n.service" | sed 's/^\s*//g' | cut -f1 -d' ') == $n.service ]]; then
        return 0
    else
        return 1
    fi
}

if service_exists lqos_node_manager; then
    /bin/systemctl disable --now lqos_node_manager # In case it's running from a previous release
    rm -f /etc/systemd/system/lqosd_node_manager.service # In case it's running from a previous release
fi

/bin/systemctl daemon-reload
/bin/systemctl enable --now lqosd lqos_scheduler
popd > /dev/null || exit

# Attempting to fixup versioning issues with libpython.
# This requires that you already have LibreQoS installed.
LINKED_PYTHON=$(ldd /opt/libreqos/src/bin/lqosd | grep libpython | sed -e '/^[^\t]/ d' | sed -e 's/\t//' | sed -e 's/.*=..//' | sed -e 's/ (0.*)//')
if ! test -f $LINKED_PYTHON; then
    if test -f /lib/x86_64-linux-gnu/libpython3.12.so.1.0; then
        ln -s /lib/x86_64-linux-gnu/libpython3.12.so.1.0 $LINKED_PYTHON
    fi
    if test -f /lib/x86_64-linux-gnu/libpython3.11.so.1.0; then
        ln -s /lib/x86_64-linux-gnu/libpython3.11.so.1.0 $LINKED_PYTHON
    fi
fi
EOF

# Uninstall Script
cat << EOF > postrm
#!/bin/bash
/bin/systemctl disable --now lqosd lqos_scheduler
/bin/systemctl daemon-reload
rm -f /etc/systemd/system/{lqosd,lqos_scheduler}.service
EOF

chmod a+x postrm postinst
popd > /dev/null || exit

# Copy files into the LibreQoS directory
cp $LQOS_FILES $LQOS_DIR

# Copy files into the LibreQoS/bin directory
cp bin/*service.example $LQOS_DIR/bin

####################################################
# Build the Rust programs
pushd rust > /dev/null || exit
cargo clean
cargo build --all --release
popd > /dev/null || exit

# Copy newly built Rust files
# - The Python integration Library
cp rust/target/release/liblqos_python.so $LQOS_DIR

# - The main executables
for prog in $RUSTPROGS
do
    cp rust/target/release/$prog $LQOS_DIR/bin
done

# Compile the website
pushd rust/lqosd > /dev/null || exit
./copy_files.sh $LQOS_DIR/bin/static2
popd || exit

####################################################
# Add Message of the Day
pushd $MOTD_DIR > /dev/null || exit
cat << 'EOF' > 99-libreqos
#!/bin/bash
MY_IP=`hostname -I | cut -d' ' -f1`
echo -e "\nLibreQoS Traffic Shaper is installed on this machine.
\nPoint a browser at http://$MY_IP:9123/ to manage it.\n"
EOF
chmod a+x 99-libreqos
popd > /dev/null || exit

####################################################
# Assemble the package
dpkg-deb --root-owner-group --build $DPKG_DIR
