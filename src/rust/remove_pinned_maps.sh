#!/bin/bash
rm -v /sys/fs/bpf/map_traffic
rm -v /sys/fs/bpf/map_ip_to_cpu_and_tc
rm -v /sys/fs/bpf/cpu_map
rm -v /sys/fs/bpf/cpus_available
rm -v /sys/fs/bpf/packet_ts
rm -v /sys/fs/bpf/flow_state
rm -v /sys/fs/bpf/rtt_tracker
rm -v /sys/fs/bpf/map_ip_to_cpu_and_tc_recip
rm -v /sys/fs/bpf/map_txq_config
rm -v /sys/fs/bpf/bifrost_interface_map
rm -v /sys/fs/bpf/bifrost_vlan_map
rm -v /sys/fs/bpf/heimdall
rm -v /sys/fs/bpf/heimdall_config
rm -v /sys/fs/bpf/heimdall_watching
rm -v /sys/fs/bpf/flowbee
rm -v /sys/fs/bpf/ip_to_cpu_and_tc_hotcache
rm -v /sys/fs/bpf/ip_mapping_epoch
