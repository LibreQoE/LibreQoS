#!/bin/bash

# Checks all Rust projects for accidental inclusion of something GPL-3
# licensed.

PROJECTS="lqos_bus lqos_config lqos_heimdall lqos_node_manager lqos_python lqos_queue_tracker lqos_setup lqos_sys lqos_utils lqosd lqusers xdp_iphash_to_cpu_cmdline xdp_pping"
TOOL="cargo license --help"

# Check that the tool exists
if ! $TOOL &> /dev/null
then
    echo "Cargo License Tool not Found. Installing it."
    cargo install cargo-license
fi

# Check every project
for project in $PROJECTS
do
    pushd $project > /dev/null
    if cargo license | grep "GPL-3"; then
        echo "Warning: GPL3 detected in dependencies for $project"
    fi
    popd > /dev/null
done