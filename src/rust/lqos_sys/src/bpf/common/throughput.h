#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "maximums.h"
#include "debug.h"

// Counter for each host
struct host_counter {
    __u64 download_bytes;
    __u64 upload_bytes;
    __u64 download_packets;
    __u64 upload_packets;
    __u32 tc_handle;
};

// Pinned map storing counters per host. its an LRU structure: if it
// runs out of space, the least recently seen host will be removed.
struct
{
	__uint(type, BPF_MAP_TYPE_LRU_PERCPU_HASH);
	__type(key, struct in6_addr);
	__type(value, struct host_counter);
    __uint(max_entries, MAX_TRACKED_IPS);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} map_traffic SEC(".maps");

static __always_inline void track_traffic(
    int direction, 
    struct in6_addr * key, 
    __u32 size, 
    __u32 tc_handle
) {
    // Count the bits. It's per-CPU, so we can't be interrupted - no sync required
    struct host_counter * counter = 
        (struct host_counter *)bpf_map_lookup_elem(&map_traffic, key);
    if (counter) {
        if (direction == 1) {
            // Download
            counter->download_packets += 1;
            counter->download_bytes += size;
            counter->tc_handle = tc_handle;
        } else {
            // Upload
            counter->upload_packets += 1;
            counter->upload_bytes += size;
            counter->tc_handle = tc_handle;
        }
    } else {
        struct host_counter new_host = {0};
        new_host.tc_handle = tc_handle;
        if (direction == 1) {
            new_host.download_packets = 1;
            new_host.download_bytes = size;
            new_host.upload_bytes = 0;
            new_host.upload_packets = 0;
        } else {
            new_host.upload_packets = 1;
            new_host.upload_bytes = size;
            new_host.download_bytes = 0;
            new_host.download_packets = 0;
        }
        if (bpf_map_update_elem(&map_traffic, key, &new_host, BPF_NOEXIST) != 0) {
            bpf_debug("Failed to insert flow");
        }
    }
}