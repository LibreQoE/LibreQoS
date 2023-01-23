#!/bin/bash

# This script builds the Rust sub-system and places the results in the
# `src/bin` directory.
#
# You still need to setup services to run `lqosd` and `lqos_node_manager`
# automatically.
#
# Don't forget to setup `/etc/lqos.conf`
PROGS="lqosd lqtop xdp_iphash_to_cpu_cmdline xdp_pping lqos_node_manager webusers"
mkdir -p bin/static
pushd rust
#cargo clean
for prog in $PROGS
do
    pushd $prog
    cargo build --release
    popd
done

for prog in $PROGS
do
    cp target/release/$prog ../bin
done
popd

# Copy the node manager's static web content
cp -R rust/lqos_node_manager/static/* bin/static

# Copy Rocket.toml to tell the node manager where to listen
cp rust/lqos_node_manager/Rocket.toml bin/

# Copy the Python library for LibreQoS.py et al.
pushd rust/lqos_python
cargo build --release
popd
cp rust/target/release/liblqos_python.so .

echo "-----------------------------------------------------------------"
echo "Don't forget to setup /etc/lqos.conf!"
echo "Template .service files can be found in bin/"
echo ""
echo "Run rust/remove_pinned_maps.sh before you restart lqosd"
echo "This ensures that any data-format changes will apply correctly."
