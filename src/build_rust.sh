#!/bin/bash

# This script builds the Rust sub-system and places the results in the
# `src/bin` directory.
#
# You still need to setup services to run `lqosd` and possibly `lqos_scheduler` automatically.
#
# Don't forget to setup `/etc/lqos.conf`

FAST_BUILD=0
for arg in "$@"
do
    case "$arg" in
        --fast)
            FAST_BUILD=1
            ;;
        *)
            echo "Unknown argument: $arg"
            echo "Usage: $0 [--fast]"
            exit 2
            ;;
    esac
done

# Check Pre-Requisites
sudo apt install python3-pip clang gcc gcc-multilib llvm libelf-dev git nano curl screen llvm pkg-config linux-tools-common linux-tools-`uname -r` libbpf-dev libssl-dev curl

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
if [ "$FAST_BUILD" -eq 1 ]; then
    BUILD_FLAGS="--profile fast-release"
    TARGET=fast-release
    echo "Using fast local iteration profile"
else
    BUILD_FLAGS=--release
    TARGET=release
fi

# Enable this if you are building on the same computer you are running on
RUSTFLAGS="-C target-cpu=native"

# Check for Rust version
echo "Checking that Rust is uptodate"
rustup update

# Start building
echo "Please wait while the system is compiled. Service will not be interrupted during this stage."
PROGS=(
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
BUILD_PACKAGES=(
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
    lqos_python
)
mkdir -p bin/static
pushd rust > /dev/null || exit
#cargo clean
PACKAGE_ARGS=()
for pkg in "${BUILD_PACKAGES[@]}"
do
    PACKAGE_ARGS+=("-p" "$pkg")
done

# If the environment variable FLAMEGRAPHS is set, lqosd needs its own build with that feature.
if [ -n "${FLAMEGRAPHS:-}" ]; then
    echo "Building lqosd with flamegraph support"
    cargo build $BUILD_FLAGS -p lqosd -F flamegraphs
    NON_LQOSD_ARGS=()
    for pkg in "${BUILD_PACKAGES[@]}"
    do
        if [ "$pkg" != "lqosd" ]; then
            NON_LQOSD_ARGS+=("-p" "$pkg")
        fi
    done
    cargo build $BUILD_FLAGS "${NON_LQOSD_ARGS[@]}"
else
    echo "Building Rust workspace binaries and lqos_python"
    cargo build $BUILD_FLAGS "${PACKAGE_ARGS[@]}"
fi
popd > /dev/null || exit

echo "Installing new binaries into bin folder."
pushd rust > /dev/null || exit
for prog in "${PROGS[@]}"
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

refresh_service_unit() {
    local unit_name=$1
    local src="./bin/${unit_name}.service.example"
    local dst="/etc/systemd/system/${unit_name}.service"
    if [ -f "$src" ] && [ -f "$dst" ]; then
        echo "Refreshing $dst from $src"
        sudo cp "$src" "$dst"
        SERVICE_UNITS_UPDATED=1
    fi
}

clear_pinned_maps_before_lqosd_restart() {
    local script_path="./rust/remove_pinned_maps.sh"
    if [ ! -x "$script_path" ]; then
        echo "Expected $script_path to exist and be executable before restarting lqosd."
        exit 1
    fi

    echo "Removing pinned BPF maps before restarting lqosd."
    if ! sudo "$script_path"; then
        echo "Failed to remove pinned maps. Skipping service restarts to avoid restarting lqosd with stale map state."
        exit 1
    fi
}

SERVICE_UNITS_UPDATED=0
refresh_service_unit lqosd
refresh_service_unit lqos_scheduler
refresh_service_unit lqos_api
refresh_service_unit lqos_setup

if [ "$SERVICE_UNITS_UPDATED" -eq 1 ]; then
    echo "Reloading systemd unit definitions."
    sudo systemctl daemon-reload
fi

if service_exists lqos_node_manager; then
    echo "lqos_node_manager is running as a service. It's not needed anymore. Killing it."
    sudo systemctl stop lqos_node_manager
    sudo systemctl disable lqos_node_manager
fi
if service_exists lqosd; then
    clear_pinned_maps_before_lqosd_restart
    echo "lqosd is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqosd
fi
if service_exists lqos_netplan_helper; then
    echo "lqos_netplan_helper is no longer run as a service. Stopping and disabling it."
    sudo systemctl stop lqos_netplan_helper || true
    sudo systemctl disable lqos_netplan_helper || true
    if [ -f /etc/systemd/system/lqos_netplan_helper.service ]; then
        sudo rm -f /etc/systemd/system/lqos_netplan_helper.service
        sudo systemctl daemon-reload
    fi
fi
if service_exists lqos_scheduler; then
    echo "lqos_scheduler is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_scheduler
fi
if service_exists lqos_api; then
    echo "lqos_api is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_api
fi
if service_exists lqos_setup; then
    echo "lqos_setup is running as a service. Restarting it. You may need to enter your sudo password."
    sudo systemctl restart lqos_setup
fi

echo "-----------------------------------------------------------------"
echo "Don't forget to setup /etc/lqos.conf!"
echo "Template .service files can be found in bin/"
echo "If src/deb-requirements-constraints.txt exists, Debian package installs use it to constrain Python dependencies."
echo "Use ./systemd_hotfix.sh to evaluate or install the Ubuntu 24.04 networkd hotfix from the LibreQoS APT repo at https://repo.libreqos.com."
echo "The hotfix installer now offers to schedule a reboot after it finishes."
echo "LibreQoS package installs on affected Ubuntu 24.04 hosts stop until the hotfix is installed; finish with sudo dpkg --configure -a after the reboot."
