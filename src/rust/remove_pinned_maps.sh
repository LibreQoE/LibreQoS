#!/bin/bash
rm -vf /sys/fs/bpf/map_traffic
rm -vf /sys/fs/bpf/map_ip_to_cpu_and_tc
rm -vf /sys/fs/bpf/cpu_map
rm -vf /sys/fs/bpf/cpus_available
rm -vf /sys/fs/bpf/packet_ts
rm -vf /sys/fs/bpf/flow_state
rm -vf /sys/fs/bpf/rtt_tracker
rm -vf /sys/fs/bpf/map_ip_to_cpu_and_tc_recip
rm -vf /sys/fs/bpf/map_txq_config
rm -vf /sys/fs/bpf/bifrost_interface_map
rm -vf /sys/fs/bpf/bifrost_vlan_map
rm -vf /sys/fs/bpf/heimdall
rm -vf /sys/fs/bpf/heimdall_config
rm -vf /sys/fs/bpf/heimdall_watching
rm -vf /sys/fs/bpf/flowbee
rm -vf /sys/fs/bpf/ip_to_cpu_and_tc_hotcache
rm -vf /sys/fs/bpf/ip_mapping_epoch
