#pragma once

#include <linux/in6.h>
#include <linux/ip.h>
#include <linux/ipv6.h>

// Provides hashing services for merging IPv4 and IPv6 addresses into
// the same memory format.

// Union that contains either a pointer to an IPv4 header or an IPv6
// header. NULL if not present.
// Note that you also need to keep track of the header type, since
// accessing it directly without checking is undefined behavior.
union iph_ptr
{
    // IPv4 Header
    struct iphdr *iph;
    // IPv6 Header
    struct ipv6hdr *ip6h;
};

// Encodes an IPv4 address into an IPv6 address. All 0xFF except for the
// last 32-bits.
static __always_inline void encode_ipv4(
    __be32 addr, 
    struct in6_addr * out_address
) {
    __builtin_memset(&out_address->in6_u.u6_addr8, 0xFF, 16);
    out_address->in6_u.u6_addr32[3] = addr;
}

// Encodes an IPv6 address into an IPv6 address. Unsurprisingly, that's
// just a memcpy operation.
static __always_inline void encode_ipv6(
    struct in6_addr * ipv6_address, 
    struct in6_addr * out_address
) {
    __builtin_memcpy(
        &out_address->in6_u.u6_addr8, 
        &ipv6_address->in6_u.u6_addr8, 
        16
    );
}