#!/bin/bash
sudo rm -v /sys/fs/bpf/map_traffic
sudo rm -v /sys/fs/bpf/map_ip_to_cpu_and_tc
sudo rm -v /sys/fs/bpf/cpu_map
sudo rm -v /sys/fs/bpf/cpus_available
sudo rm -v /sys/fs/bpf/packet_ts
sudo rm -v /sys/fs/bpf/flow_state
sudo rm -v /sys/fs/bpf/rtt_tracker
sudo rm -v /sys/fs/bpf/map_ip_to_cpu_and_tc_recip
sudo rm -v /sys/fs/bpf/tc/globals/map_txq_config
sudo rm -v /sys/fs/bpf/bifrost_interface_map
sudo rm -v /sys/fs/bpf/bifrost_vlan_map

