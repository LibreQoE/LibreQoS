#!/bin/bash

# This script builds the Rust sub-system and places the results in the
# `src/bin` directory.
#
# You still need to setup services to run `lqosd` and possibly `lqos_scheduler` automatically.
#
# Don't forget to setup `/etc/lqos.conf`

# Check Pre-Requisites
sudo apt install python3-pip clang gcc gcc-multilib llvm libelf-dev git nano graphviz curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev libssl-dev curl

if ! rustup -V &> /dev/null
then
    echo "rustup is not installed."
    echo "Visit https://rustup.rs and install Rust from there"
    echo "Usually, you can copy the following and follow the on-screen instructions."
    echo "Please don't install Rust as root."
    echo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
else
    echo "rustup found."
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
PROGS="lqosd lqtop xdp_iphash_to_cpu_cmdline xdp_pping lqusers lqos_map_perf uisp_integration lqos_overrides"
mkdir -p bin/static
pushd rust > /dev/null || exit
#cargo clean
for prog in $PROGS
do
    # If prog is lqosd
    if [ $prog == "lqosd" ]; then
        # If the environment variable FLAMEGRAPHS is set, set the FEATURE variable to flamegraph, otherwise it's empty
        if [ -n "$FLAMEGRAPHS" ]; then
            echo "Building lqosd with flamegraph support"
            FEATURE="-F flamegraphs"
        else
            echo "Building lqosd without flamegraph support"
            FEATURE=""
        fi
        echo "Building lqosd"
        pushd lqosd > /dev/null || exit
        cargo build $BUILD_FLAGS $FEATURE
        if [ $? -ne 0 ]; then
          echo "Cargo build failed. Exiting with code 1."
          exit 1
        fi
        popd > /dev/null || exit
    else
      pushd $prog > /dev/null || exit
      cargo build $BUILD_FLAGS
      if [ $? -ne 0 ]; then
        echo "Cargo build failed. Exiting with code 1."
        exit 1
      fi
      popd || exit
    fi
done
popd > /dev/null || exit

echo "Installing new binaries into bin folder."
pushd rust > /dev/null || exit
for prog in $PROGS
do
    echo "Installing $prog in bin folder"
    cp target/$TARGET/$prog ../bin/$prog.new || exit
    # Use a move to avoid file locking
    mv ../bin/$prog.new ../bin/$prog || exit
done
popd > /dev/null || exit

# Copy the node manager's static web content
mkdir -p bin/static2/vendor
pushd rust/lqosd > /dev/null || exit
./copy_files.sh
popd > /dev/null || exit

# Copy the Python library for LibreQoS.py et al.
pushd rust/lqos_python > /dev/null || exit
cargo build $BUILD_FLAGS
popd > /dev/null || exit
cp rust/target/$TARGET/liblqos_python.so ./liblqos_python.so.new
mv liblqos_python.so.new liblqos_python.so



# Update the lqos_api binary
echo "Updating lqos_api binary..."
bash ./update_api.sh || echo "Warning: Failed to update lqos_api (continuing)."

# If we're running systemd, we need to restart processes
service_exists() {
    local n=$1
    if [[ $(systemctl list-units --all -t service --full --no-legend "$n.service" | sed 's/^\s*//g' | cut -f1 -d' ') == $n.service ]]; then
        return 0
    else
        return 1
    fi
}

if service_exists lqos_node_manager; then
    echo "lqos_node_manager is running as a service. It's not needed anymore. Killing it."
    sudo systemctl stop lqos_node_manager
    sudo systemctl disable lqos_node_manager
fi
if service_exists lqosd; then
    echo "lqosd is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqosd
fi
if service_exists lqos_scheduler; then
    echo "lqos_scheduler is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_scheduler
fi
if service_exists lqos_api; then
    echo "lqos_api is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_api
fi

echo "-----------------------------------------------------------------"
echo "Don't forget to setup /etc/lqos.conf!"
echo "Template .service files can be found in bin/"
echo ""
echo "Run sudo rust/remove_pinned_maps.sh before you restart lqosd"
echo "This ensures that any data-format changes will apply correctly."
