#!/bin/bash

# This script builds the Rust sub-system and places the results in the
# `src/bin` directory.
#
# You still need to setup services to run `lqosd` and `lqos_node_manager`
# automatically.
#
# Don't forget to setup `/etc/lqos`
PROGS="lqosd lqtop xdp_iphash_to_cpu_cmdline xdp_pping lqos_node_manager"
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

echo "Don't forget to setup /etc/lqos!"
echo "Template .service files can be found in bin/"
