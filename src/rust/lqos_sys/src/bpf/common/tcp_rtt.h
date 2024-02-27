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
 
/* Implementation of pping inside the kernel
 * classifier
 */
#ifndef __TC_CLASSIFY_KERN_PPING_H
#define __TC_CLASSIFY_KERN_PPING_H

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <linux/pkt_cls.h>
#include <linux/in.h>
#include <linux/in6.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include <linux/tcp.h>
#include <bpf/bpf_endian.h>
#include <stdbool.h>
#include "tc_classify_kern_pping_common.h"
#include "maximums.h"
#include "debug.h"
#include "ip_hash.h"
#include "dissector_tc.h"
#include "tcp_opts.h"

#define MAX_MEMCMP_SIZE 128

struct parsing_context
{
    struct tcphdr *tcp;
    __u64 now;
    struct tc_dissector_t * dissector;
    struct in6_addr * active_host;
};

/* Event type recorded for a packet flow */
enum __attribute__((__packed__)) flow_event_type
{
    FLOW_EVENT_NONE,
    FLOW_EVENT_OPENING,
    FLOW_EVENT_CLOSING,
    FLOW_EVENT_CLOSING_BOTH
};

enum __attribute__((__packed__)) connection_state
{
    CONNECTION_STATE_EMPTY,
    CONNECTION_STATE_WAITOPEN,
    CONNECTION_STATE_OPEN,
    CONNECTION_STATE_CLOSED
};

struct flow_state
{
    __u64 last_timestamp;
    __u32 last_id;
    __u32 outstanding_timestamps;
    enum connection_state conn_state;
    __u8 reserved[2];
};

/*
 * Stores flowstate for both direction (src -> dst and dst -> src) of a flow
 *
 * Uses two named members instead of array of size 2 to avoid hassels with
 * convincing verifier that member access is not out of bounds
 */
struct dual_flow_state
{
    struct flow_state dir1;
    struct flow_state dir2;
};

/*
 * Struct filled in by parse_packet_id.
 *
 * Note: As long as parse_packet_id is successful, the flow-parts of pid
 * and reply_pid should be valid, regardless of value for pid_valid and
 * reply_pid valid. The *pid_valid members are there to indicate that the
 * identifier part of *pid are valid and can be used for timestamping/lookup.
 * The reason for not keeping the flow parts as an entirely separate members
 * is to save some performance by avoid doing a copy for lookup/insertion
 * in the packet_ts map.
 */
struct packet_info
{
    __u64 time; // Arrival time of packet
    //__u32 payload;              // Size of packet data (excluding headers)
    struct packet_id pid;       // flow + identifier to timestamp (ex. TSval)
    struct packet_id reply_pid; // rev. flow + identifier to match against (ex. TSecr)
    //__u32 ingress_ifindex;      // Interface packet arrived on (if is_ingress, otherwise not valid)    
    bool pid_flow_is_dfkey;              // Used to determine which member of dualflow state to use for forward direction
    bool pid_valid;                      // identifier can be used to timestamp packet
    bool reply_pid_valid;                // reply_identifier can be used to match packet
    enum flow_event_type event_type;     // flow event triggered by packet
};

/*
 * Struct filled in by protocol id parsers (ex. parse_tcp_identifier)
 */
struct protocol_info
{
    __u32 pid;
    __u32 reply_pid;
    bool pid_valid;
    bool reply_pid_valid;
    enum flow_event_type event_type;
};



/* Map Definitions */
struct
{
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __type(key, struct packet_id);
    __type(value, __u64);
    __uint(max_entries, MAX_PACKETS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
//    __uint(map_flags, BPF_F_NO_PREALLOC);
} packet_ts SEC(".maps");

struct
{
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __type(key, struct network_tuple);
    __type(value, struct dual_flow_state);
    __uint(max_entries, MAX_FLOWS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
//    __uint(map_flags, BPF_F_NO_PREALLOC);
} flow_state SEC(".maps");

struct
{
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __type(key, struct in6_addr); // Keyed to the IP address
    __type(value, struct rotating_performance);
    __uint(max_entries, IP_HASH_ENTRIES_MAX);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
//    __uint(map_flags, BPF_F_NO_PREALLOC);

} rtt_tracker SEC(".maps");

// Mask for IPv6 flowlabel + traffic class -  used in fib lookup
#define IPV6_FLOWINFO_MASK __cpu_to_be32(0x0FFFFFFF)

#ifndef AF_INET
#define AF_INET 2
#endif
#ifndef AF_INET6
#define AF_INET6 10
#endif

#define MAX_TCP_OPTIONS 10

/* Functions */

/*
 * Convenience function for getting the corresponding reverse flow.
 * PPing needs to keep track of flow in both directions, and sometimes
 * also needs to reverse the flow to report the "correct" (consistent
 * with Kathie's PPing) src and dest address.
 */
static __always_inline void reverse_flow(
    struct network_tuple *dest, 
    struct network_tuple *src
) {
    dest->ipv = src->ipv;
    dest->proto = src->proto;
    dest->saddr = src->daddr;
    dest->daddr = src->saddr;
    dest->reserved = 0;
}

/*
 * Can't seem to get __builtin_memcmp to work, so hacking my own
 *
 * Based on https://githubhot.com/repo/iovisor/bcc/issues/3559,
 * __builtin_memcmp should work constant size but I still get the "failed to
 * find BTF for extern" error.
 */
static __always_inline int my_memcmp(
    const void *s1_, 
    const void *s2_, 
    __u32 size
) {
    const __u8 *s1 = (const __u8 *)s1_, *s2 = (const __u8 *)s2_;
    int i;

    for (i = 0; i < MAX_MEMCMP_SIZE && i < size; i++)
    {
        if (s1[i] != s2[i])
            return s1[i] > s2[i] ? 1 : -1;
    }

    return 0;
}

static __always_inline bool is_dualflow_key(struct network_tuple *flow)
{
    return my_memcmp(&flow->saddr, &flow->daddr, sizeof(flow->saddr)) <= 0;
}

static __always_inline struct flow_state *fstate_from_dfkey(
    struct dual_flow_state *df_state,
    bool is_dfkey
) {
    if (!df_state) {
        return (struct flow_state *)NULL;
    }

    return is_dfkey ? &df_state->dir1 : &df_state->dir2;
}

/*
 * Attempts to fetch an identifier for TCP packets, based on the TCP timestamp
 * option.
 *
 * Will use the TSval as pid and TSecr as reply_pid, and the TCP source and dest
 * as port numbers.
 *
 * If successful, tcph, sport, dport and proto_info will be set
 * appropriately and 0 will be returned.
 * On failure -1 will be returned (and arguments will not be set).
 */
static __always_inline int parse_tcp_identifier(
    struct parsing_context *context,
    __u16 *sport,
    __u16 *dport, 
    struct protocol_info *proto_info
) {
    if (parse_tcp_ts(context->tcp, context->dissector->end, &proto_info->pid, 
        &proto_info->reply_pid) < 0) {
        return -1; // Possible TODO, fall back on seq/ack instead
    }

    // Do not timestamp pure ACKs (no payload)
    void *nh_pos = (context->tcp + 1) + (context->tcp->doff << 2);
    proto_info->pid_valid = nh_pos - context->dissector->start < context->dissector->ctx->len || context->tcp->syn;

    // Do not match on non-ACKs (TSecr not valid)
    proto_info->reply_pid_valid = context->tcp->ack;

    // Check if connection is opening/closing
    if (context->tcp->rst)
    {
        proto_info->event_type = FLOW_EVENT_CLOSING_BOTH;
    }
    else if (context->tcp->fin)
    {
        proto_info->event_type = FLOW_EVENT_CLOSING;
    }
    else if (context->tcp->syn)
    {
        proto_info->event_type = FLOW_EVENT_OPENING;
    }
    else
    {
        proto_info->event_type = FLOW_EVENT_NONE;
    }

    *sport = bpf_ntohs(context->tcp->dest);
    *dport = bpf_ntohs(context->tcp->source);

    return 0;
}

/* This is a bit of a hackjob from the original */
static __always_inline int parse_packet_identifier(
    struct parsing_context *context, 
    struct packet_info *p_info
) {
    p_info->time = context->now;
    if (context->dissector->eth_type == ETH_P_IP)
    {
        p_info->pid.flow.ipv = AF_INET;
        p_info->pid.flow.saddr.ip = context->dissector->src_ip;
        p_info->pid.flow.daddr.ip = context->dissector->dst_ip;
    }
    else if (context->dissector->eth_type == ETH_P_IPV6)
    {
        p_info->pid.flow.ipv = AF_INET6;
        p_info->pid.flow.saddr.ip = context->dissector->src_ip;
        p_info->pid.flow.daddr.ip = context->dissector->dst_ip;
    }
    else
    {
        bpf_debug("Unknown protocol");
        return -1;
    }
    //bpf_debug("IPs: %u %u", p_info->pid.flow.saddr.ip.in6_u.u6_addr32[3], p_info->pid.flow.daddr.ip.in6_u.u6_addr32[3]);

    struct protocol_info proto_info;
    int err = parse_tcp_identifier(context,
                                   &p_info->pid.flow.saddr.port,
                                   &p_info->pid.flow.daddr.port,
                                   &proto_info);
    if (err)
        return -1;
    //bpf_debug("Ports: %u %u", p_info->pid.flow.saddr.port, p_info->pid.flow.daddr.port);

    // Sucessfully parsed packet identifier - fill in remaining members and return
    p_info->pid.identifier = proto_info.pid;
    p_info->pid_valid = proto_info.pid_valid;
    p_info->reply_pid.identifier = proto_info.reply_pid;
    p_info->reply_pid_valid = proto_info.reply_pid_valid;
    p_info->event_type = proto_info.event_type;

    if (p_info->pid.flow.ipv == AF_INET && p_info->pid.flow.ipv == AF_INET6) {
        bpf_debug("Unknown internal protocol");
        return -1;
    }

    p_info->pid_flow_is_dfkey = is_dualflow_key(&p_info->pid.flow);

    reverse_flow(&p_info->reply_pid.flow, &p_info->pid.flow);

    return 0;
}

static __always_inline struct network_tuple *
get_dualflow_key_from_packet(struct packet_info *p_info)
{
    return p_info->pid_flow_is_dfkey ? &p_info->pid.flow : &p_info->reply_pid.flow;
}

/*
 * Initilizes an "empty" flow state based on the forward direction of the
 * current packet
 */
static __always_inline void init_flowstate(struct flow_state *f_state,
                                           struct packet_info *p_info)
{
    f_state->conn_state = CONNECTION_STATE_WAITOPEN;
    f_state->last_timestamp = p_info->time;
}

static __always_inline void init_empty_flowstate(struct flow_state *f_state)
{
    f_state->conn_state = CONNECTION_STATE_EMPTY;
}

static __always_inline struct flow_state *
get_flowstate_from_packet(struct dual_flow_state *df_state,
                          struct packet_info *p_info)
{
    return fstate_from_dfkey(df_state, p_info->pid_flow_is_dfkey);
}

static __always_inline struct flow_state *
get_reverse_flowstate_from_packet(struct dual_flow_state *df_state,
                                  struct packet_info *p_info)
{
    return fstate_from_dfkey(df_state, !p_info->pid_flow_is_dfkey);
}

/*
 * Initilize a new (assumed 0-initlized) dual flow state based on the current
 * packet.
 */
static __always_inline void init_dualflow_state(
    struct dual_flow_state *df_state,
    struct packet_info *p_info
) {
    struct flow_state *fw_state =
        get_flowstate_from_packet(df_state, p_info);
    struct flow_state *rev_state =
        get_reverse_flowstate_from_packet(df_state, p_info);

    init_flowstate(fw_state, p_info);
    init_empty_flowstate(rev_state);
}

static __always_inline struct dual_flow_state *
create_dualflow_state(
    struct parsing_context *ctx, 
    struct packet_info *p_info, 
    bool *new_flow
) {
    struct network_tuple *key = get_dualflow_key_from_packet(p_info);
    struct dual_flow_state new_state = {0};

    init_dualflow_state(&new_state, p_info);
    //new_state.dir1.tc_handle.handle = ctx->tc_handle;
    //new_state.dir2.tc_handle.handle = ctx->tc_handle;

    if (bpf_map_update_elem(&flow_state, key, &new_state, BPF_NOEXIST) ==
        0)
    {
        if (new_flow)
            *new_flow = true;
    }
    else
    {
        return (struct dual_flow_state *)NULL;
    }

    return (struct dual_flow_state *)bpf_map_lookup_elem(&flow_state, key);
}

static __always_inline struct dual_flow_state *
lookup_or_create_dualflow_state(
    struct parsing_context *ctx, 
    struct packet_info *p_info, 
    bool *new_flow
) {
    struct dual_flow_state *df_state;

    struct network_tuple *key = get_dualflow_key_from_packet(p_info);
    df_state = (struct dual_flow_state *)bpf_map_lookup_elem(&flow_state, key);

    if (df_state)
    {
        return df_state;
    }

    // Only try to create new state if we have a valid pid
    if (!p_info->pid_valid || p_info->event_type == FLOW_EVENT_CLOSING ||
        p_info->event_type == FLOW_EVENT_CLOSING_BOTH)
        return (struct dual_flow_state *)NULL;

    return create_dualflow_state(ctx, p_info, new_flow);
}

static __always_inline bool is_flowstate_active(struct flow_state *f_state)
{
    return f_state->conn_state != CONNECTION_STATE_EMPTY &&
           f_state->conn_state != CONNECTION_STATE_CLOSED;
}

static __always_inline void update_forward_flowstate(
    struct packet_info *p_info,
    struct flow_state *f_state, 
    bool *new_flow
) {
    // "Create" flowstate if it's empty
    if (f_state->conn_state == CONNECTION_STATE_EMPTY &&
        p_info->pid_valid)
    {
        init_flowstate(f_state, p_info);
        if (new_flow)
            *new_flow = true;
    }
}

static __always_inline void update_reverse_flowstate(
    void *ctx, 
    struct packet_info *p_info,
    struct flow_state *f_state
) {
    if (!is_flowstate_active(f_state))
        return;

    // First time we see reply for flow?
    if (f_state->conn_state == CONNECTION_STATE_WAITOPEN &&
        p_info->event_type != FLOW_EVENT_CLOSING_BOTH)
    {
        f_state->conn_state = CONNECTION_STATE_OPEN;
    }
}

static __always_inline bool is_new_identifier(
    struct packet_id *pid, 
    struct flow_state *f_state
) {
    if (pid->flow.proto == IPPROTO_TCP)
        /* TCP timestamps should be monotonically non-decreasing
         * Check that pid > last_ts (considering wrap around) by
         * checking 0 < pid - last_ts < 2^31 as specified by
         * RFC7323 Section 5.2*/
        return pid->identifier - f_state->last_id > 0 &&
               pid->identifier - f_state->last_id < 1UL << 31;

    return pid->identifier != f_state->last_id;
}

static __always_inline bool is_rate_limited(__u64 now, __u64 last_ts)
{
    if (now < last_ts)
        return true;

    // Static rate limit
    //return now - last_ts < DELAY_BETWEEN_RTT_REPORTS_MS * NS_PER_MS;
    return false; // Max firehose drinking speed
}

/*
 * Attempt to create a timestamp-entry for packet p_info for flow in f_state
 */
static __always_inline void pping_timestamp_packet(
    struct flow_state *f_state, 
    void *ctx,
    struct packet_info *p_info, 
    bool new_flow
) {
    if (!is_flowstate_active(f_state) || !p_info->pid_valid)
        return;

    // Check if identfier is new
    if (!new_flow && !is_new_identifier(&p_info->pid, f_state))
        return;
    f_state->last_id = p_info->pid.identifier;

    // Check rate-limit
    if (!new_flow && is_rate_limited(p_info->time, f_state->last_timestamp))
        return;

    /*
     * Updates attempt at creating timestamp, even if creation of timestamp
     * fails (due to map being full). This should make the competition for
     * the next available map slot somewhat fairer between heavy and sparse
     * flows.
     */
    f_state->last_timestamp = p_info->time;

    if (bpf_map_update_elem(&packet_ts, &p_info->pid, &p_info->time,
                            BPF_NOEXIST) == 0)
        __sync_fetch_and_add(&f_state->outstanding_timestamps, 1);
}

/*
 * Attempt to match packet in p_info with a timestamp from flow in f_state
 */
static __always_inline void pping_match_packet(struct flow_state *f_state,
                                               struct packet_info *p_info,
                                               struct in6_addr *active_host)
{
    __u64 *p_ts;

    if (!is_flowstate_active(f_state) || !p_info->reply_pid_valid)
        return;

    if (f_state->outstanding_timestamps == 0)
        return;

    p_ts = (__u64 *)bpf_map_lookup_elem(&packet_ts, &p_info->reply_pid);
    if (!p_ts || p_info->time < *p_ts)
        return;

    __u64 rtt = (p_info->time - *p_ts) / NS_PER_MS_TIMES_100;
    bpf_debug("RTT (from TC): %u", p_info->time - *p_ts);

    // Delete timestamp entry as soon as RTT is calculated
    if (bpf_map_delete_elem(&packet_ts, &p_info->reply_pid) == 0)
    {
        __sync_fetch_and_add(&f_state->outstanding_timestamps, -1);
    }

    // Update the most performance map to include this data
    struct rotating_performance *perf = 
        (struct rotating_performance *)bpf_map_lookup_elem(
            &rtt_tracker, active_host);
    if (perf == NULL) return;
    __sync_fetch_and_add(&perf->next_entry, 1);
    __u32 next_entry = perf->next_entry;
    if (next_entry < MAX_PERF_SECONDS) {
        __sync_fetch_and_add(&perf->rtt[next_entry], rtt);
        perf->has_fresh_data = 1;
    }
}

static __always_inline void close_and_delete_flows(
    void *ctx, 
    struct packet_info *p_info,
    struct flow_state *fw_flow,
    struct flow_state *rev_flow
) {
    // Forward flow closing
    if (p_info->event_type == FLOW_EVENT_CLOSING ||
        p_info->event_type == FLOW_EVENT_CLOSING_BOTH)
    {
        fw_flow->conn_state = CONNECTION_STATE_CLOSED;
    }

    // Reverse flow closing
    if (p_info->event_type == FLOW_EVENT_CLOSING_BOTH)
    {
        rev_flow->conn_state = CONNECTION_STATE_CLOSED;
    }

    // Delete flowstate entry if neither flow is open anymore
    if (!is_flowstate_active(fw_flow) && !is_flowstate_active(rev_flow))
    {
        bpf_map_delete_elem(&flow_state, get_dualflow_key_from_packet(p_info));
    }
}

/*
 * Contains the actual pping logic that is applied after a packet has been
 * parsed and deemed to contain some valid identifier.
 * Looks up and updates flowstate (in both directions), tries to save a
 * timestamp of the packet, tries to match packet against previous timestamps,
 * calculates RTTs and pushes messages to userspace as appropriate.
 */
static __always_inline void pping_parsed_packet(
    struct parsing_context *context, 
    struct packet_info *p_info
) {
    struct dual_flow_state *df_state;
    struct flow_state *fw_flow, *rev_flow;
    bool new_flow = false;

    df_state = lookup_or_create_dualflow_state(context, p_info, &new_flow);
    if (!df_state)
    {
        // bpf_debug("No flow state - stop");
        return;
    }

    fw_flow = get_flowstate_from_packet(df_state, p_info);
    update_forward_flowstate(p_info, fw_flow, &new_flow);
    pping_timestamp_packet(fw_flow, context, p_info, new_flow);

    rev_flow = get_reverse_flowstate_from_packet(df_state, p_info);
    update_reverse_flowstate(context, p_info, rev_flow);
    pping_match_packet(rev_flow, p_info, context->active_host);

    close_and_delete_flows(context, p_info, fw_flow, rev_flow);
}

/* Entry poing for running pping in the tc context */
static __always_inline void tc_pping_start(struct parsing_context *context)
{
    // Check to see if we can store perf info. Bail if we've hit the limit.
    // Copying occurs because otherwise the validator complains.
    struct rotating_performance *perf = 
        (struct rotating_performance *)bpf_map_lookup_elem(
            &rtt_tracker, context->active_host);
    if (perf) {
        if (perf->next_entry >= MAX_PERF_SECONDS-1) {
            //bpf_debug("Flow has max samples. Not sampling further until next reset.");
            //for (int i=0; i<MAX_PERF_SECONDS; ++i) {
            //    bpf_debug("%u", perf->rtt[i]);
            //}
            if (context->now > perf->recycle_time) {
                // If the time-to-live for the sample is exceeded, recycle it to be
                // usable again.
                //bpf_debug("Recycling flow, %u > %u", context->now, perf->recycle_time);
                __builtin_memset(perf->rtt, 0, sizeof(__u32) * MAX_PERF_SECONDS);
                perf->recycle_time = context->now + RECYCLE_RTT_INTERVAL;
                perf->next_entry = 0;
                perf->has_fresh_data = 0;
            }
            return;
        }
    }

    // Populate the TCP Header
    if (context->dissector->eth_type == ETH_P_IP)
    {
        // If its not TCP, stop
        if (context->dissector->ip_header.iph + 1 > context->dissector->end)
            return; // Stops the error checking from crashing
        if (context->dissector->ip_header.iph->protocol != IPPROTO_TCP)
        {
            return;
        }
        context->tcp = (struct tcphdr *)((char *)context->dissector->ip_header.iph + (context->dissector->ip_header.iph->ihl * 4));
    }
    else if (context->dissector->eth_type == ETH_P_IPV6)
    {
        // If its not TCP, stop
        if (context->dissector->ip_header.ip6h + 1 > context->dissector->end)
            return; // Stops the error checking from crashing
        if (context->dissector->ip_header.ip6h->nexthdr != IPPROTO_TCP)
        {
            return;
        }
        context->tcp = (struct tcphdr *)(context->dissector->ip_header.ip6h + 1);
    }
    else
    {
        bpf_debug("UNKNOWN PROTOCOL TYPE");
        return;
    }

    // Bail out if the packet is incomplete
    if (context->tcp + 1 > context->dissector->end)
    {
        return;
    }

    // If we didn't get a handle, make one
    if (perf == NULL)
    {
        struct rotating_performance new_perf = {0};
        new_perf.recycle_time = context->now + RECYCLE_RTT_INTERVAL;
        new_perf.has_fresh_data = 0;
        if (bpf_map_update_elem(&rtt_tracker, context->active_host, &new_perf, BPF_NOEXIST) != 0) return;
    }


    // Start the parsing process
    struct packet_info p_info = {0};
    if (parse_packet_identifier(context, &p_info) < 0)
    {
        //bpf_debug("Unable to parse packet identifier");
        return;
    }

    pping_parsed_packet(context, &p_info);
}

#endif /* __TC_CLASSIFY_KERN_PPING_H */

/*

Understanding how this works (psuedocode):

1. Parsing context is passed into tc_pping_start
    1. We lookup the rotating_performance map for the active host (local side).
        1. If it exists, we check to see if we are in "next entry" time window yet.
        2. If we are, and the current time exceeds the "recycle time", we reset the
           performance map and set the "recycle time" to the current time plus the
           recycle interval. We exit the function.
    2. We then check to see if the packet is TCP. If it is not, we exit the function.
    3. We then check to see if the packet is complete. If it is not, we exit the function.
    4. We then parse the packet identifier. If we are unable to parse the packet identifier,
       we exit the function. (the `parse_packet_identifier` function).
        1. We set the packet time to the current time.
        2. We set the flow type to either AF_INET or AF_INET6.
        3. We set the source and destination IP addresses.
        4. We call `parse_tcp_identifier` to parse the TCP identifier.
            1. We use `parse_tcp_ts` to extract the TSval and TSecr from the TCP header.
               These are stored in `proto_info.pid` and `proto_info.reply_pid`.
               If we fail to parse the TCP identifier, we exit the function.
            2. We set "pid_valid" to true if the next header position is less than the end of the packet
               or if the packet is a SYN packet. (i.e. ignore packets with no payload).
            3. We set "reply_pid_valid" to true if the packet is an ACK packet.
            4. RST events are set to "FLOW_EVENT_CLOSING_BOTH", FIN events are set to "FLOW_EVENT_CLOSING",
               and SYN events are set to "FLOW_EVENT_OPENING".
            5. We set the source and destination ports.
        5. If we failed to parse the TCP identifier, we exit the function.
        6. We set "pid.identifier" to "proto_info.pid" and "reply_pid.identifier" to "proto_info.reply_pid".
        7. We set "pid_valid" to "proto_info.pid_valid" and "reply_pid_valid" to "proto_info.reply_pid_valid".
        8. We set "event_type" to "proto_info.event_type".
        9. We bail if the protocol is not AF_INET or AF_INET6.
        10. We set "pid_flow_is_dfkey" to "is_dualflow_key(&p_info->pid.flow)".
            1. Compare the source and destination addresses and return true when it
                encounters a packet with the source address less than the destination address.
            2. This appears to be a way to sort the flow keys.
        11. We call `reverse_flow` with the reply flow and the forward flow.
            1.Reverse flow sets the destination to the source.
    5. We then call pping_parsed_packet with the parsing context and the packet info.
        1. We call `lookup_or_create_dualflow_state` and return it if we found one.
            1. We call `get_dualflow_key_from_packet` to get the flow key from the packet.
                1.
            2. If `pid_valid` is false, or the event type is "FLOW_EVENT_CLOSING" or "FLOW_EVENT_CLOSING_BOTH",
               we return NULL.
            3. If we still haven't got a flow state, we call `create_dualflow_state` with the parsing context,
               the packet info, and a pointer to new_flow.
                1. We call `get_dualflow_key_from_packet` to get the flow key from the packet.
                    1. If "pid_flow_is_dfkey" we return pid.flow, otherwise reply_pid.flow.
                2. We call `init_dualflow_state` with the new state and the packet info.
                3. We create a new state in the flow state map (or return an existing one).
            4. We set `fw_flow` with `get_flowstate_from_packet` and the packet info.
                1. This in turns calls `fstate_from_dfkey` with the dual flow state and the packet info.
                    1. If the packet flow is the dual flow key, we return dir1, otherwise dir2.
            5. We call `update_forward_flowstate` with the packet info.
                1. If the connection state is empty and the packet identifier is valid, we call `init_flowstate`
                   with the flow state and the packet info.
                   1. `init_flowstate` sets the connection state to "WAITOPEN" and the last timestamp to the packet time.
            6. We call `pping_timestamp_packet` with the forward flow, the parsing context, the packet info, and new_flow.
                1. If the flow state is not active, or the packet identifier is not valid, we return.
                2. If the flow state is not new and the identifier is not new, we return.
                3. If the flow state is not new and the packet is rate limited, we return.
                4. We set the last timestamp to the packet time.
            7. We set `rev_flow` with `get_reverse_flowstate_from_packet` and the packet info.
                1.
            8. We call `update_reverse_flowstate` with the parsing context, the packet info, and the reverse flow.
                1.
            9. We call `pping_match_packet` with the reverse flow, the packet info, and the active host.
                1. If the flow state is not active, or the reply packet identifier is not valid, we return.
                2. If the flow state has no outstanding timestamps, we return.
                3. We call `bpf_map_lookup_elem` with the packet timestamp map and the reply packet identifier.
                    1. If the lookup fails, or the packet time is less than the timestamp, we return.
                4. We calculate the round trip time.
                5. We call `bpf_map_delete_elem` with the packet timestamp map and the reply packet identifier.
                    1. If the delete is successful, we decrement the outstanding timestamps.
            10. We call `close_and_delete_flows` with the parsing context, the packet info, the forward flow, and the reverse flow.
                1.
*/