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
cp -R rust/lqos_node_manager/static/* bin/static
cp rust/lqos_node_manager/Rocket.toml bin/
echo "Don't forget to setup /etc/lqos!"
