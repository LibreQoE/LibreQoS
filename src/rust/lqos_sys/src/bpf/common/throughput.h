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
    __u64 tcp_download_packets;
    __u64 tcp_upload_packets;
    __u64 udp_download_packets;
    __u64 udp_upload_packets;
    __u64 icmp_download_packets;
    __u64 icmp_upload_packets;
    __u32 tc_handle;
    __u64 circuit_id;
    __u64 device_id;
    __u64 last_seen;
};

// Pinned map storing counters per host. its an LRU structure: if it
// runs out of space, the least recently seen host will be removed.
struct
{
    __uint(type, BPF_MAP_TYPE_PERCPU_HASH);
    __type(key, struct in6_addr);
    __type(value, struct host_counter);
    __uint(max_entries, MAX_TRACKED_IPS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} map_traffic SEC(".maps");

// Scratch space to avoid large host_counter allocations on the stack
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct host_counter);
} map_traffic_scratch SEC(".maps");

static __always_inline void track_traffic(
    int direction, 
    struct in6_addr * key, 
    __u32 size, 
    __u32 tc_handle,
    __u64 circuit_id,
    __u64 device_id,
    struct dissector_t * dissector
) {
    // Count the bits. It's per-CPU, so we can't be interrupted - no sync required
    struct host_counter * counter = 
        (struct host_counter *)bpf_map_lookup_elem(&map_traffic, key);
    if (counter) {
        counter->last_seen = dissector->now;
        counter->tc_handle = tc_handle;
        counter->circuit_id = circuit_id;
        counter->device_id = device_id;
        if (direction == 1) {
            // Download
            counter->download_packets += 1;
            counter->download_bytes += size;
            switch (dissector->ip_protocol) {
                case IPPROTO_TCP:
                    counter->tcp_download_packets += 1;
                    break;
                case IPPROTO_UDP:
                    counter->udp_download_packets += 1;
                    break;
                case IPPROTO_ICMP:
                    counter->icmp_download_packets += 1;
                    break;
            }
        } else {
            // Upload
            counter->upload_packets += 1;
            counter->upload_bytes += size;
            switch (dissector->ip_protocol) {
                case IPPROTO_TCP:
                    counter->tcp_upload_packets += 1;
                    break;
                case IPPROTO_UDP:
                    counter->udp_upload_packets += 1;
                    break;
                case IPPROTO_ICMP:
                    counter->icmp_upload_packets += 1;
                    break;
            }
        }
    } else {
        __u32 zero = 0;
        struct host_counter *new_host = bpf_map_lookup_elem(&map_traffic_scratch, &zero);
        if (!new_host) return;
        __builtin_memset(new_host, 0, sizeof(*new_host));
        new_host->tc_handle = tc_handle;
        new_host->circuit_id = circuit_id;
        new_host->device_id = device_id;
        new_host->last_seen = dissector->now;
        if (direction == 1) {
            new_host->download_packets = 1;
            new_host->download_bytes = size;
            switch (dissector->ip_protocol) {
                case IPPROTO_TCP:
                    new_host->tcp_download_packets = 1;
                    break;
                case IPPROTO_UDP:
                    new_host->udp_download_packets = 1;
                    break;
                case IPPROTO_ICMP:
                    new_host->icmp_download_packets = 1;
                    break;
            }
        } else {
            new_host->upload_packets = 1;
            new_host->upload_bytes = size;
            switch (dissector->ip_protocol) {
                case IPPROTO_TCP:
                    new_host->tcp_upload_packets = 1;
                    break;
                case IPPROTO_UDP:
                    new_host->udp_upload_packets = 1;
                    break;
                case IPPROTO_ICMP:
                    new_host->icmp_upload_packets = 1;
                    break;
            }
        }
        if (bpf_map_update_elem(&map_traffic, key, new_host, BPF_NOEXIST) != 0) {
            bpf_debug("Failed to insert flow");
        }
    }
}
