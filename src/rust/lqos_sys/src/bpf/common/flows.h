/* SPDX-License-Identifier: GPL-2.0 */

// TCP flow monitor system

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include "dissector.h"
#include "debug.h"


#define SECOND_IN_NANOS 1000000000
#define TWO_SECONDS_IN_NANOS 2000000000
#define MS_IN_NANOS_T10 10000
#define HALF_MBPS_IN_BYTES_PER_SECOND 62500
#define RTT_RING_SIZE 4
//#define TIMESTAMP_INTERVAL_NANOS 10000000

// Some helpers to make understanding direction easier
// for readability.
#define TO_INTERNET 2
#define FROM_INTERNET 1
#define TO_LOCAL 1
#define FROM_LOCAL 2

// Defines a TCP connection flow key
struct flow_key_t {
    struct in6_addr src;
    struct in6_addr dst;
    __u16 src_port;
    __u16 dst_port;
    __u8 protocol;
    __u8 pad;
    __u8 pad1;
    __u8 pad2;
};

// TCP connection flow entry
struct flow_data_t {
    // Time (nanos) when the connection was established
    __u64 start_time;
    // Time (nanos) when the connection was last seen
    __u64 last_seen;
    // Bytes transmitted
    __u64 bytes_sent[2];
    // Packets transmitted
    __u64 packets_sent[2];
    // Clock for the next rate estimate
    __u64 next_count_time[2];
    // Clock for the previous rate estimate
    __u64 last_count_time[2];
    // Bytes at the next rate estimate
    __u64 next_count_bytes[2];
    // Rate estimate
    __u32 rate_estimate_bps[2];
    // Sequence number of the last packet
    __u32 last_sequence[2];
    // Acknowledgement number of the last packet
    __u32 last_ack[2];
    // Retransmit Counters (Also catches duplicates and out-of-order packets)
    __u16 tcp_retransmits[2];
    // Timestamp values
    __u32 tsval[2];
    __u32 tsecr[2];
    // When did the timestamp change?
    __u64 ts_change_time[2];
    // Has the connection ended?
    // 0 = Alive, 1 = FIN, 2 = RST
    __u8 end_status;
    // TOS
    __u8 tos;
    // IP Flags
    __u8 ip_flags;
    // Padding
    __u8 pad;
};

// Map for tracking TCP flow progress.
// This is pinned and not per-CPU, because half the data appears on either side of the bridge.
struct
{
    __uint(type, BPF_MAP_TYPE_HASH); // TODO: BPF_MAP_TYPE_LRU_PERCPU_HASH?
    __type(key, struct flow_key_t);
    __type(value, struct flow_data_t);
    __uint(max_entries, MAX_FLOWS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} flowbee SEC(".maps");

// Ringbuffer to userspace for recording RTT events
struct {
	__uint(type, BPF_MAP_TYPE_RINGBUF);
	__uint(max_entries, 256 * 1024 /* 256 KB */);
} flowbee_events SEC(".maps");

// Event structure we send for events.
struct flowbee_event {
    struct flow_key_t key;
    __u64 round_trip_time;
    __u32 effective_direction;
};

// Construct an empty flow_data_t structure, using default values.
static __always_inline struct flow_data_t new_flow_data(
    // The packet dissector from the previous step
    struct dissector_t *dissector
) {
    struct flow_data_t data = {
        .start_time = dissector->now,
        .bytes_sent = { 0, 0 },
        .packets_sent = { 0, 0 },
        // Track flow rates at an MS scale rather than per-second
        // to minimize rounding errors.
        .next_count_time = { dissector->now + SECOND_IN_NANOS, dissector->now + SECOND_IN_NANOS },
        .last_count_time = { dissector->now, dissector->now },
        .next_count_bytes = { 0, 0 }, // Should be packet size, that isn't working?
        .rate_estimate_bps = { 0, 0 },
        .last_sequence = { 0, 0 },
        .last_ack = { 0, 0 },
        .tcp_retransmits = { 0, 0 },
        .tsval = { 0, 0 },
        .tsecr = { 0, 0 },
        .ts_change_time = { 0, 0 },
        .end_status = 0,
        .tos = 0,
        .ip_flags = 0,
    };
    return data;
}

// Construct a flow_key_t structure from a dissector_t. This represents the
// unique key for a flow in the flowbee map.
static __always_inline struct flow_key_t build_flow_key(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    __u16 src_port = direction == FROM_INTERNET ? bpf_htons(dissector->src_port) : bpf_htons(dissector->dst_port);
    __u16 dst_port = direction == FROM_INTERNET ? bpf_htons(dissector->dst_port) : bpf_htons(dissector->src_port);
    struct in6_addr src = direction == FROM_INTERNET ? dissector->src_ip : dissector->dst_ip;
    struct in6_addr dst = direction == FROM_INTERNET ? dissector->dst_ip : dissector->src_ip;

    return (struct flow_key_t) {
        .src = src,
        .dst = dst,
        .src_port = src_port,
        .dst_port = dst_port,
        .protocol = dissector->ip_protocol,
        .pad = 0,
        .pad1 = 0,
        .pad2 = 0
    };
}

// Update the flow data with the current packet's information.
// * Update the timestamp of the last seen packet
// * Update the bytes and packets sent
// * Update the rate estimate (if it is time to do so)
static __always_inline void update_flow_rates(
    // The packet dissector from the previous step
    struct dissector_t *dissector,
    // The rate index (0 = to internet, 1 = to local network)
    u_int8_t rate_index,
    // The flow data structure to update
    struct flow_data_t *data
) {
    data->last_seen = dissector->now;
    data->end_status = 0; // Reset the end status

    // Update bytes and packets sent
    data->bytes_sent[rate_index] += dissector->skb_len;
    data->packets_sent[rate_index]++;

    if (dissector->now > data->next_count_time[rate_index]) {
        // Calculate the rate estimate
        __u64 bits = (data->bytes_sent[rate_index] - data->next_count_bytes[rate_index])*8;
        __u64 time = (dissector->now - data->last_count_time[rate_index]) / SECOND_IN_NANOS; // 1 Second
        data->rate_estimate_bps[rate_index] = (bits/time); // bits per second
        data->next_count_time[rate_index] = dissector->now + SECOND_IN_NANOS;
        data->next_count_bytes[rate_index] = data->bytes_sent[rate_index];
        data->last_count_time[rate_index] = dissector->now;
        //bpf_debug("[FLOWS] Rate Estimate: %llu", data->rate_estimate_bps[rate_index]);
    }
}

// Handle Per-Flow ICMP Analysis
static __always_inline void process_icmp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int8_t rate_index,
    u_int8_t other_rate_index
) {
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // There isn't a flow, so we need to make one
        struct flow_data_t new_data = new_flow_data(dissector);
        if (bpf_map_update_elem(&flowbee, &key, &new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        data = bpf_map_lookup_elem(&flowbee, &key);
        if (data == NULL) return;
    }
    update_flow_rates(dissector, rate_index, data);
}

// Handle Per-Flow UDP Analysis
static __always_inline void process_udp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int8_t rate_index,
    u_int8_t other_rate_index
) {
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // There isn't a flow, so we need to make one
        struct flow_data_t new_data = new_flow_data(dissector);
        if (bpf_map_update_elem(&flowbee, &key, &new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        data = bpf_map_lookup_elem(&flowbee, &key);
        if (data == NULL) return;
    }
    update_flow_rates(dissector, rate_index, data);
}

// Store the most recent sequence and ack numbers, and detect retransmissions.
// This will also trigger on duplicate packets, and out-of-order - but those
// are both an indication that you have issues anyway. So that's ok by me!
static __always_inline void detect_retries(
    struct dissector_t *dissector,
    u_int8_t rate_index,
    struct flow_data_t *data
) {
    __u32 sequence = bpf_ntohl(dissector->sequence);
    __u32 ack_seq = bpf_ntohl(dissector->ack_seq);
    if (
        data->last_sequence[rate_index] != 0 && // We have a previous sequence number
        sequence < data->last_sequence[rate_index] && // This is a retransmission
        (
            data->last_sequence[rate_index] > 0x10000 && // Wrap around possible
            sequence > data->last_sequence[rate_index] - 0x10000 // Wrap around didn't occur            
        ) 
    ) {
        // This is a retransmission
        data->tcp_retransmits[rate_index]++;
    }

    // Store the sequence and ack numbers for the next packet
    data->last_sequence[rate_index] = sequence;
    data->last_ack[rate_index] = ack_seq;
}

// Handle Per-Flow TCP Analysis
static __always_inline void process_tcp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int8_t rate_index,
    u_int8_t other_rate_index
) {
    // SYN packet indicating the start of a conversation. We are explicitly ignoring
    // SYN-ACK packets, we just want to catch the opening of a new connection.
    if ((BITCHECK(DIS_TCP_SYN) && !BITCHECK(DIS_TCP_ACK) && direction == TO_INTERNET) || 
        (BITCHECK(DIS_TCP_SYN) && !BITCHECK(DIS_TCP_ACK) && direction == FROM_INTERNET)) {
        // A customer is requesting a new TCP connection. That means
        // we need to start tracking this flow.
        #ifdef VERBOSE
        bpf_debug("[FLOWS] New TCP Connection Detected (%u)", direction);
        #endif
        struct flow_key_t key = build_flow_key(dissector, direction);
        struct flow_data_t data = new_flow_data(dissector);
        data.tos = dissector->tos;
        data.ip_flags = 0; // Obtain these
        if (bpf_map_update_elem(&flowbee, &key, &data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
        }
        return;
    }

    // Build the flow key to uniquely identify this flow
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // If it isn't a flow we're tracking, bail out now
        #ifdef VERBOSE
        bpf_debug("Bailing");
        #endif
        return;
    }

    // Update the flow data with the current packet's information
    update_flow_rates(dissector, rate_index, data);

    // Sequence and Acknowledgement numbers
    detect_retries(dissector, rate_index, data);

    // Timestamps to calculate RTT
    if (dissector->tsval != 0) {
        //bpf_debug("[FLOWS][%d] TSVAL: %u, TSECR: %u", direction, tsval, tsecr);
        if (dissector->tsval != data->tsval[rate_index] && dissector->tsecr != data->tsecr[rate_index]) {

            if (
                dissector->tsecr == data->tsval[other_rate_index] &&
                (data->rate_estimate_bps[rate_index] > 0 ||
                data->rate_estimate_bps[other_rate_index] > 0 )
            ) {
                __u64 elapsed = dissector->now - data->ts_change_time[other_rate_index];
                if (elapsed < TWO_SECONDS_IN_NANOS) {
                    struct flowbee_event event = { 0 };
                    event.key = key;
                    event.round_trip_time = elapsed;
                    event.effective_direction = rate_index;
                    bpf_ringbuf_output(&flowbee_events, &event, sizeof(event), 0);
                }
            }

            data->ts_change_time[rate_index] = dissector->now;
            data->tsval[rate_index] = dissector->tsval;
            data->tsecr[rate_index] = dissector->tsecr;
        }
    }

    // Has the connection ended?
    if (BITCHECK(DIS_TCP_FIN)) {
        data->end_status = 1;
    } else if (BITCHECK(DIS_TCP_RST)) {
        data->end_status = 2;
    }
}

// Note that this duplicates a lot of what we do for "snoop" - we're hoping
// to replace both it and the old RTT system.
static __always_inline void track_flows(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    u_int8_t rate_index;
    u_int8_t other_rate_index;
    if (direction == TO_INTERNET) {
        rate_index = 0;
        other_rate_index = 1;
    } else {
        rate_index = 1;
        other_rate_index = 0;
    }

    // Pass to the appropriate protocol handler
    switch (dissector->ip_protocol)
    {
        case IPPROTO_TCP: process_tcp(dissector, direction, rate_index, other_rate_index); break;
        case IPPROTO_UDP: process_udp(dissector, direction, rate_index, other_rate_index); break;
        case IPPROTO_ICMP: process_icmp(dissector, direction, rate_index, other_rate_index); break;
        default: {
            #ifdef VERBOSE
            bpf_debug("[FLOWS] Unsupported protocol: %d", dissector->ip_protocol);
            #endif
        }
    }
}
