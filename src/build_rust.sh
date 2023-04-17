#!/bin/bash

# This script builds the Rust sub-system and places the results in the
# `src/bin` directory.
#
# You still need to setup services to run `lqosd` and `lqos_node_manager`
# automatically.
#
# Don't forget to setup `/etc/lqos.conf`

# Check Pre-Requisites
if ! bpftool help &> /dev/null
then
    echo "bpftool is not installed."
    echo "Let's try to install it"
    sudo apt-get install linux-tools-common linux-tools-`uname -r`
else
    echo "bpftool found."
fi

if ! pkg-config --help &> /dev/null
then
    echo "pkg-config is not installed."
    echo "Let's try to install it"
    sudo apt-get install pkg-config
else
    echo "pkg-config found."
fi

if ! clang -v &> /dev/null
then
    echo "LLVM/clang is not installed."
    echo "Let's try to install it"
    sudo apt-get install llvm libelf-dev gcc gcc-multilib libbpf-dev
else
    echo "LLVM/clang found."
fi

# To enable heavy debug mode (slow)
#BUILD_FLAGS=""
#TARGET=debug
# Otherwise
BUILD_FLAGS=--release
TARGET=release

# Enable this if you are building on the same computer you are running on
RUSTFLAGS="-C target-cpu=native"

# Check for Rust version
echo "Checking that Rust is uptodate"
rustup update

# Start building
echo "Please wait while the system is compiled. Service will not be interrupted during this stage."
PROGS="lqosd lqtop xdp_iphash_to_cpu_cmdline xdp_pping lqos_node_manager lqusers lqos_map_perf"
mkdir -p bin/static
pushd rust > /dev/null
#cargo clean
for prog in $PROGS
do
    pushd $prog > /dev/null
    cargo build $BUILD_FLAGS
    popd > /dev/null
done

echo "Installing new binaries into bin folder."
for prog in $PROGS
do
    echo "Installing $prog in bin folder"
    cp target/$TARGET/$prog ../bin/$prog.new
    # Use a move to avoid file locking
    mv ../bin/$prog.new ../bin/$prog
done
popd > /dev/null

# Copy the node manager's static web content
cp -R rust/lqos_node_manager/static/* bin/static

# Copy Rocket.toml to tell the node manager where to listen
cp rust/lqos_node_manager/Rocket.toml bin/

# Copy the Python library for LibreQoS.py et al.
pushd rust/lqos_python > /dev/null
cargo build $BUILD_FLAGS
popd > /dev/null
cp rust/target/$TARGET/liblqos_python.so ./liblqos_python.so.new
mv liblqos_python.so.new liblqos_python.so

# If we're running systemd, we need to restart processes
service_exists() {
    local n=$1
    if [[ $(systemctl list-units --all -t service --full --no-legend "$n.service" | sed 's/^\s*//g' | cut -f1 -d' ') == $n.service ]]; then
        return 0
    else
        return 1
    fi
}

if service_exists lqosd; then
    echo "lqosd is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqosd
fi
if service_exists lqos_node_manager; then
    echo "lqos_node_manager is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_node_manager
fi

echo "-----------------------------------------------------------------"
echo "Don't forget to setup /etc/lqos.conf!"
echo "Template .service files can be found in bin/"
echo ""
echo "Run sudo rust/remove_pinned_maps.sh before you restart lqosd"
echo "This ensures that any data-format changes will apply correctly."
