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
#include "debug.h"
#include "dissector.h"
#include "dissector_tc.h"

// Data structure used for map_ip_hash
struct ip_hash_info {
	__u32 cpu;
	__u32 tc_handle; // TC handle MAJOR:MINOR combined in __u32
};

// Key type used for map_ip_hash trie
struct ip_hash_key {
	__u32 prefixlen; // Length of the prefix to match
	struct in6_addr address; // An IPv6 address. IPv4 uses the last 32 bits.
};

// Map describing IP to CPU/TC mappings
struct {
	__uint(type, BPF_MAP_TYPE_LPM_TRIE);
	__uint(max_entries, IP_HASH_ENTRIES_MAX);
	__type(key, struct ip_hash_key);
	__type(value, struct ip_hash_info);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
	__uint(map_flags, BPF_F_NO_PREALLOC);
} map_ip_to_cpu_and_tc SEC(".maps");

// RECIPROCAL Map describing IP to CPU/TC mappings
// If in "on a stick" mode, this is used to
// fetch the UPLOAD mapping.
struct {
	__uint(type, BPF_MAP_TYPE_LPM_TRIE);
	__uint(max_entries, IP_HASH_ENTRIES_MAX);
	__type(key, struct ip_hash_key);
	__type(value, struct ip_hash_info);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
	__uint(map_flags, BPF_F_NO_PREALLOC);
} map_ip_to_cpu_and_tc_recip SEC(".maps");

// Performs an LPM lookup for an `ip_hash.h` encoded address, taking
// into account redirection and "on a stick" setup.
static __always_inline struct ip_hash_info * setup_lookup_key_and_tc_cpu(
    // The "direction" constant from the main program. 1 = Internet,
    // 2 = LAN, 3 = Figure it out from VLAN tags
    int direction,
    // Pointer to the "lookup key", which should contain the IP address
    // to search for. Prefix length will be set for you.
    struct ip_hash_key * lookup_key,
    // Pointer to the traffic dissector.
    struct dissector_t * dissector,
    // Which VLAN represents the Internet, in redirection scenarios? (i.e.
    // when direction == 3)
    __be16 internet_vlan,
    // Out variable setting the real "direction" of traffic when it has to
    // be calculated.
    int * out_effective_direction
) 
{
    lookup_key->prefixlen = 128;
    // Normal preset 2-interface setup, no need to calculate any direction
    // related VLANs.
    if (direction < 3) {
        lookup_key->address = (direction == 1) ? dissector->dst_ip : 
            dissector->src_ip;
        *out_effective_direction = direction;
        struct ip_hash_info * ip_info = bpf_map_lookup_elem(
            &map_ip_to_cpu_and_tc, 
            lookup_key
        );
        return ip_info;
    } else {
        if (dissector->current_vlan == internet_vlan) {
            // Packet is coming IN from the Internet.
            // Therefore it is download.
            lookup_key->address = dissector->dst_ip;
            *out_effective_direction = 1;
            struct ip_hash_info * ip_info = bpf_map_lookup_elem(
                &map_ip_to_cpu_and_tc, 
                lookup_key
            );
            return ip_info;
        } else {
            // Packet is coming IN from the ISP.
            // Therefore it is UPLOAD.
            lookup_key->address = dissector->src_ip;
            *out_effective_direction = 2;
            struct ip_hash_info * ip_info = bpf_map_lookup_elem(
                &map_ip_to_cpu_and_tc_recip, 
                lookup_key
            );
            return ip_info;
        }
    }
}

// For the TC side, the dissector is different. Operates similarly to
// `setup_lookup_key_and_tc_cpu`. Performs an LPM lookup for an `ip_hash.h` 
// encoded address, taking into account redirection and "on a stick" setup.
static __always_inline struct ip_hash_info * tc_setup_lookup_key_and_tc_cpu(
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
        return ip_info;
    } else {
        //bpf_debug("Current VLAN (TC): %d", dissector->current_vlan);
        //bpf_debug("Source: %x", dissector->src_ip.in6_u.u6_addr32[3]);
        //bpf_debug("Dest: %x", dissector->dst_ip.in6_u.u6_addr32[3]);
        if (dissector->current_vlan == internet_vlan) {
            // Packet is going OUT to the Internet.
            // Therefore, it is UPLOAD.
            lookup_key->address = dissector->src_ip;
            *out_effective_direction = 2;
            //bpf_debug("Reciprocal lookup");
            struct ip_hash_info * ip_info = bpf_map_lookup_elem(
                &map_ip_to_cpu_and_tc_recip, 
                lookup_key
            );
            return ip_info;
        } else {
            // Packet is going OUT to the LAN.
            // Therefore, it is DOWNLOAD.
            lookup_key->address = dissector->dst_ip;
            *out_effective_direction = 1;
            //bpf_debug("Forward lookup");
            struct ip_hash_info * ip_info = bpf_map_lookup_elem(
                &map_ip_to_cpu_and_tc, 
                lookup_key
            );
            return ip_info;
        }
    }
    struct ip_hash_info * ip_info = bpf_map_lookup_elem(
        &map_ip_to_cpu_and_tc, 
        lookup_key
    );
    return ip_info;
}