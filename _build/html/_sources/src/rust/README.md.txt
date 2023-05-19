# Rust Management System for LibreQoS

> Very much a work in progress. Details will be filled out as it stabilizes.

## Sub Projects

This project contains a number of projects arranged in a workspace. The projects are:

* `lqos_sys` - a library that builds, installs, removes and manages the LibreQoS XDP and TC programs.
* `lqos_bus` - definitions and helper functions for passing data across the local management bus.
* `lqos_config` - a crate that handles pulling configuration from the Python manager.
* `lqosd` - the management daemon that should eventually be run as a `systemd` service.
    * When started, the daemon sets up XDP/TC eBPF programs for the interfaces specified in the LibreQoS configuration.
    * When exiting, all eBPF programs are unloaded.
    * Listens for bus commands and applies them.
* `lqtop` - A CLI tool that outputs the top X downloaders and mostly verifies that the bus and daemons work.
* `xdp_iphash_to_cpu_cmdline` - An almost-compatible command that acts like the tool of the same name from the previous verion.
* `xdp_pping` - Port of the previous release's `xdp_pping` tool, for compatibility. Will eventually not be needed.

## Required Ubuntu packages

* `clang`
* `linux-tools-common` (for `bpftool`)
* `libbpf-dev`
* `gcc-multilib`
* `llvm`
* `pkg-config`
* `linux-tools-5.15.0-56-generic` (the common version doesn't work?)

## Helper Scripts

* `remove_pinned_maps.sh` deletes all of the BPF shared maps. Useful during development.
