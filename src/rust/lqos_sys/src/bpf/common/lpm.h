#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include <linux/in6.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include "maximums.h"
#include "dissector.h"

// Data structure used for map_ip_hash
struct ip_hash_info {
	__u32 cpu;
	__u32 tc_handle; // TC handle MAJOR:MINOR combined in __u32
	__u64 circuit_id;
	__u64 device_id;
};

// In on-a-stick mode, upload classes/CPUs are offset by this amount.
// This is configured by userspace at load time.
extern __u32 stick_offset;

// Epoch used to notify the dataplane that IP->TC/CPU mappings have changed.
// Userspace bumps this (and clears the hot cache) after applying mapping updates.
// Flowbee uses it to refresh per-flow cached mapping metadata only when needed.
struct {
	__uint(type, BPF_MAP_TYPE_ARRAY);
	__uint(max_entries, 1);
	__type(key, __u32);
	__type(value, __u32);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} ip_mapping_epoch SEC(".maps");

// Key type used for map_ip_hash trie
struct ip_hash_key {
	__u32 prefixlen; // Length of the prefix to match
	struct in6_addr address; // An IPv6 address. IPv4 uses the last 32 bits.
};

// Hot cache for recent IP lookups, an attempt
// at a speed improvement predicated on the idea
// that LPM isn't the fastest
// The cache is optional. define USE_HOTCACHE
// to enable it.
#define USE_HOTCACHE 1

#ifdef USE_HOTCACHE
struct {
	__uint(type, BPF_MAP_TYPE_LRU_HASH);
	__uint(max_entries, HOT_CACHE_SIZE);
	__type(key, struct in6_addr);
	__type(value, struct ip_hash_info);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} ip_to_cpu_and_tc_hotcache SEC(".maps");
#endif

// Map describing IP to CPU/TC mappings
struct {
	__uint(type, BPF_MAP_TYPE_LPM_TRIE);
	__uint(max_entries, IP_HASH_ENTRIES_MAX);
	__type(key, struct ip_hash_key);
	__type(value, struct ip_hash_info);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
	__uint(map_flags, BPF_F_NO_PREALLOC);
} map_ip_to_cpu_and_tc SEC(".maps");

// Determine the effective direction of a packet
static __always_inline u_int8_t determine_effective_direction(int direction, __be16 internet_vlan, struct dissector_t * dissector) {
    if (direction < 3) {
        return direction;
    } else {
        if (dissector->current_vlan == internet_vlan) {
            return 1;
        } else {
            return 2;
        }
    }
}

static __always_inline void apply_stick_offset_to_mapping(
    u_int8_t effective_direction,
    struct ip_hash_info *mapping
) {
    // Only applies to on-a-stick mode, where upload direction is derived
    // by offsetting CPU and TC major.
    if (stick_offset == 0) {
        return;
    }
    if (effective_direction != 2) {
        return;
    }
    // If it isn't shaped, don't transform it.
    if (mapping->tc_handle == 0) {
        return;
    }

    mapping->cpu += stick_offset;
    mapping->tc_handle += stick_offset << 16;
}

// Performs an LPM lookup for an `ip_hash.h` encoded address, taking
// into account redirection and "on a stick" setup.
static __always_inline struct ip_hash_info * setup_lookup_key_and_tc_cpu(
    // This must have been pre-calculated by `determine_effective_direction`.
    u_int8_t direction,
    // Pointer to the "lookup key", which should contain the IP address
    // to search for. Prefix length will be set for you.
    struct ip_hash_key * lookup_key,
    // Pointer to the traffic dissector.
    struct dissector_t * dissector
) 
{
    struct ip_hash_info * ip_info;

    lookup_key->address = (direction == 1) ? dissector->dst_ip :
        dissector->src_ip;

    #ifdef USE_HOTCACHE
    // Try a hot cache search
    ip_info = bpf_map_lookup_elem(
        &ip_to_cpu_and_tc_hotcache,
        &lookup_key->address
    );
    if (ip_info) {
        // Is it a negative hit?
        if (ip_info->cpu == NEGATIVE_HIT) {
            return NULL;
        }

        // We got a cache hit, so return
        return ip_info;
    }
    #endif

    lookup_key->prefixlen = 128;
    ip_info = bpf_map_lookup_elem(
        &map_ip_to_cpu_and_tc, 
        lookup_key
    );
    #ifdef USE_HOTCACHE
    if (ip_info) {
        // We found it, so add it to the cache
        bpf_map_update_elem(
            &ip_to_cpu_and_tc_hotcache,
            &lookup_key->address,
            ip_info,
            BPF_NOEXIST
        );
    } else {
        // Store a negative result. This is designed to alleviate the pain
        // of repeatedly hitting queries for IPs that ARE NOT shaped.
        struct ip_hash_info negative_hit = {
            .cpu = NEGATIVE_HIT,
            .tc_handle = NEGATIVE_HIT,
            .circuit_id = 0,
            .device_id = 0,
        };
        bpf_map_update_elem(
            &ip_to_cpu_and_tc_hotcache,
            &lookup_key->address,
            &negative_hit,
            BPF_NOEXIST
        );
    }
    #endif
    return ip_info;
}

// For the TC side, the dissector is different. Operates similarly to
// `setup_lookup_key_and_tc_cpu`. Performs an LPM lookup for an `ip_hash.h` 
// encoded address, taking into account redirection and "on a stick" setup.
static __always_inline struct ip_hash_info tc_setup_lookup_key_and_tc_cpu(
    // The "direction" constant from the main program. 1 = Internet,
    // 2 = LAN, 3 = Figure it out from VLAN tags
    int direction,
    // Pointer to the "lookup key", which should contain the IP address
    // to search for. Prefix length will be set for you.
    struct ip_hash_key * lookup_key, 
    // Pointer to the traffic dissector.
    struct tc_dissector_t * dissector,
    // Which VLAN represents the Internet, in redirection scenarios? (i.e.
    // when direction == 3)
    __be16 internet_vlan,
    // Out variable setting the real "direction" of traffic when it has to
    // be calculated.
    int * out_effective_direction
) 
{
    struct ip_hash_info out = {0};
    lookup_key->prefixlen = 128;
	// Direction is reversed because we are operating on egress
    if (direction < 3) {
        lookup_key->address = (direction == 1) ? dissector->src_ip :
            dissector->dst_ip;
        *out_effective_direction = direction;

        struct ip_hash_info * ip_info = bpf_map_lookup_elem(
            &map_ip_to_cpu_and_tc, 
            lookup_key
        );
        if (ip_info) {
            out = *ip_info;
        }
        apply_stick_offset_to_mapping(*out_effective_direction, &out);
        return out;
    } else {
        //bpf_debug("Current VLAN (TC): %d", dissector->current_vlan);
        //bpf_debug("Source: %x", dissector->src_ip.in6_u.u6_addr32[3]);
        //bpf_debug("Dest: %x", dissector->dst_ip.in6_u.u6_addr32[3]);
        if (dissector->current_vlan == internet_vlan) {
            // Packet is going OUT to the Internet.
            // Therefore, it is UPLOAD.
            lookup_key->address = dissector->src_ip;
            *out_effective_direction = 2;
        } else {
            // Packet is going OUT to the LAN.
            // Therefore, it is DOWNLOAD.
            lookup_key->address = dissector->dst_ip;
            *out_effective_direction = 1;
        }

        // Regardless of effective direction, we look up the base mapping in the
        // primary map. Upload mapping is derived via stick_offset.
        struct ip_hash_info * ip_info = bpf_map_lookup_elem(
            &map_ip_to_cpu_and_tc, 
            lookup_key
        );
        if (ip_info) {
            out = *ip_info;
        }
        apply_stick_offset_to_mapping(*out_effective_direction, &out);
        return out;
    }
}
