/* SPDX-License-Identifier: GPL-2.0 */
/*
Based on the GPLv2 xdp-pping project 
(https://github.com/xdp-project/bpf-examples/tree/master/pping)

xdp_pping is based on the ideas in Dr. Kathleen Nichols' pping
utility: https://github.com/pollere/pping
   and the papers around "Listening to Networks":
http://www.pollere.net/Pdfdocs/ListeningGoog.pdf

My modifications are Copyright 2022, Herbert Wolverson
(Bracket Productions)
*/
/* Shared structures between userspace and kernel space
 */
#ifndef __TC_CLASSIFY_KERN_PPING_COMMON_H
#define __TC_CLASSIFY_KERN_PPING_COMMON_H

/* 30 second rotating performance buffer, per-TC handle */
#define MAX_PERF_SECONDS 60
#define NS_PER_MS            1000000UL
#define NS_PER_MS_TIMES_100    10000UL
#define NS_PER_SECOND NS_PER_MS 1000000000UL
#define RECYCLE_RTT_INTERVAL 10000000000UL

/* Quick way to access a TC handle as either two 16-bit numbers or a single u32 */
union tc_handle_type
{
    __u32 handle;
    __u16 majmin[2];
};

/*
 * Struct that can hold the source or destination address for a flow (l3+l4).
 * Works for both IPv4 and IPv6, as IPv4 addresses can be mapped to IPv6 ones
 * based on RFC 4291 Section 2.5.5.2.
 */
struct flow_address
{
    struct in6_addr ip;
    __u16 port;
    __u16 reserved;
};

/*
 * Struct to hold a full network tuple
 * The ipv member is technically not necessary, but makes it easier to
 * determine if saddr/daddr are IPv4 or IPv6 address (don't need to look at the
 * first 12 bytes of address). The proto memeber is not currently used, but
 * could be useful once pping is extended to work for other protocols than TCP.
 *
 * Note that I've removed proto, ipv and reserved.
 */
struct network_tuple
{
    struct flow_address saddr;
    struct flow_address daddr;
    __u16 proto; // IPPROTO_TCP, IPPROTO_ICMP, QUIC etc
    __u8 ipv;    // AF_INET or AF_INET6
    __u8 reserved;
};

/* Packet identifier */
struct packet_id
{
    struct network_tuple flow;
    __u32 identifier;
};

/* Ring-buffer of performance readings for each TC handle */
struct rotating_performance
{
    __u32 rtt[MAX_PERF_SECONDS];
    __u32 next_entry;
    __u64 recycle_time;
    __u32 has_fresh_data;
};

#endif /* __TC_CLASSIFY_KERN_PPING_COMMON_H */