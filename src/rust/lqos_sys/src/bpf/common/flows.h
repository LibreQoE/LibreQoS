/* SPDX-License-Identifier: GPL-2.0 */

// TCP flow monitor system

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include "dissector.h"
#include "debug.h"


#define SECOND_IN_NANOS 1000000000ULL
#define MS_IN_NANOS_T10 10000
#define HALF_MBPS_IN_BYTES_PER_SECOND 62500
#define RTT_RING_SIZE 4
//#define TIMESTAMP_INTERVAL_NANOS 10000000
#define TIMEOUT_TSVAL_NS (10 * SECOND_IN_NANOS)
#define MIN_RTT_SAMPLE_INTERVAL (SECOND_IN_NANOS / 10)

// Some helpers to make understanding direction easier
// for readability.
#define TO_INTERNET 2
#define FROM_INTERNET 1
#define TO_LOCAL 1
#define FROM_LOCAL 2

#ifndef ARRAY_SIZE
#define ARRAY_SIZE(arr) (sizeof(arr) / sizeof(arr[0]))
#endif

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

struct tsval_record_buffer_t {
    // Times when TSvals were observed
    // If an entry is 0 is means the spot is free
    __u64 timestamps[2];
    // The corresponding TSvals that were observed
    // tsval[i] is only valid if timestamp[i] > 0
    __u32 tsvals[2];
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
    // Retransmit Counters (Also catches duplicates and out-of-order packets)
    __u16 tcp_retransmits[2];
    // Padding to avoid 4 byte hole and push TSval/TSecr data to its own cacheline
    // Would probably be better to increase the tcp_retransmit counters to u32
    // instead, but that requires additional changes to all the user-space Rust
    // code that use them.
    __u32 pad1;
    // Timestamp values
    __u32 tsval[2];
    __u32 tsecr[2];
    // When did the timestamp change?
    struct tsval_record_buffer_t tsval_tstamps[2];
    // Last time we pushed an RTT sample
    __u64 last_rtt[2];
    // Has the connection ended?
    // 0 = Alive, 1 = FIN, 2 = RST
    __u8 end_status;
    // TOS
    __u8 tos;
    // IP Flags
    __u8 ip_flags;
    // Padding
    __u8 pad2[5];
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

// Scratch space to avoid large flow_data_t allocations on the stack
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct flow_data_t);
} flowbee_scratch SEC(".maps");

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
static __always_inline void init_flow_data(
    // The packet dissector from the previous step
    struct dissector_t *dissector,
    struct flow_data_t *data
) {
    __builtin_memset(data, 0, sizeof(*data));
    data->start_time = dissector->now;
    data->tos = dissector->tos;
    // Track flow rates at an MS scale rather than per-second
    // to minimize rounding errors.
    data->next_count_time[0] = dissector->now + SECOND_IN_NANOS;
    data->next_count_time[1] = dissector->now + SECOND_IN_NANOS;
    data->last_count_time[0] = dissector->now;
    data->last_count_time[1] = dissector->now;
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

// Checks if a < b considering u32 wraparound (logic from RFC 7323 Section 5.2)
static __always_inline bool u32wrap_lt(
    __u32 a,
    __u32 b)
{
    return a != b && b - a < 1UL << 31;
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
        __u64 time = dissector->now - data->last_count_time[rate_index]; // time in ns
        data->rate_estimate_bps[rate_index] = (bits * SECOND_IN_NANOS) / time; // nanobits per ns -> bits per second
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
        __u32 zero = 0;
        struct flow_data_t *new_data = bpf_map_lookup_elem(&flowbee_scratch, &zero);
        if (!new_data) return;
        init_flow_data(dissector, new_data);
        update_flow_rates(dissector, rate_index, new_data);
        if (bpf_map_update_elem(&flowbee, &key, new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        return;
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
        __u32 zero = 0;
        struct flow_data_t *new_data = bpf_map_lookup_elem(&flowbee_scratch, &zero);
        if (!new_data) return;
        init_flow_data(dissector, new_data);
        update_flow_rates(dissector, rate_index, new_data);
        if (bpf_map_update_elem(&flowbee, &key, new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        return;
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
    if (
        data->last_sequence[rate_index] != 0 && // We have a previous sequence number
        u32wrap_lt(sequence, data->last_sequence[rate_index]) // sequence number regression
    ) {
        // This is a retransmission
        data->tcp_retransmits[rate_index]++;
    } else {
        // Only update seq number if it's not retrans/out of order (i.e. it advances)
        data->last_sequence[rate_index] = sequence;
    }

    // Store the sequence and ack numbers for the next packet
}

static __always_inline int get_tcp_segment_size(
    struct dissector_t *dissector
) {
    struct tcphdr *tcph;
    char *payload_start;

    tcph = get_tcp_header(dissector);
    if (!tcph || tcph + 1 > dissector->end)
        return -1;

    payload_start = (char *)tcph + tcph->doff * 4;
    if (payload_start < (char *)(tcph + 1) || payload_start > dissector->end)
        return -1;

    return (char *)dissector->end - payload_start;
}

// Add a TSval <-> timestamp mapping to buf.
// Will overwrite outdated (timed out) entries.
// Will return 0 on success, or -1 if there was no free slot in buf.
static __always_inline int record_tsval(
    struct tsval_record_buffer_t *buf,
    __u64 time,
    __u32 tsval
) {
    int i;

    for (i = 0; i < ARRAY_SIZE(buf->timestamps); i++) {
        if (
            buf->timestamps[i] == 0 || // This spot has no recorded TSval
            buf->timestamps[i] + TIMEOUT_TSVAL_NS < time // This spot has an old/stale recorded TSval
        ) {
            buf->timestamps[i] = time;
            buf->tsvals[i] = tsval;
            return 0;
        }
    }

    return -1;
}

// Check if tsval has any matching recorded entry in buf.
// Will clear any outdated entries, as well as the entry it matches in buf
// On success, return the time the matched TSval was recorded.
// Return 0 if no matching entry was found.
static __always_inline __u64 match_and_clear_recorded_tsval(
    struct tsval_record_buffer_t *buf,
    __u32 tsval
) {
    __u64 match_at_time = 0;
    int i;

    for (i = 0; i < ARRAY_SIZE(buf->timestamps); i++) {
        if (buf->timestamps[i] == 0)
            // Empty entry
            continue;

        if (buf->tsvals[i] == tsval) {
            // Match - return time of match and clear out entry
            match_at_time = buf->timestamps[i];
            buf->timestamps[i] = 0;
	    // No early return to let is also clear out old entries
        } else if (u32wrap_lt(buf->tsvals[i], tsval)) {
            // Old TSval which we've already passed - clear out
            buf->timestamps[i] = 0;
        }
    }

    return match_at_time;
}

// Passively infer TCP RTT by matching ACKs to previous TCP segments using TCP
// timestamps (TSval/TSecr).
// Stores previous TSval value and checks if TSecr of current packet matches a
// previously sent TSval in the reverse direction and calculate the RTT as
// the time since the original TSval was sent. The approach is based on Kathleen
// Nichols' pping (https://pollere.net/pping.html), but modified to store
// TSvals as part of the flow state (the data argument).
static __always_inline void infer_tcp_rtt(
    struct dissector_t *dissector,
    struct flow_key_t *key,
    struct flow_data_t *data,
    u_int8_t rate_index,
    u_int8_t other_rate_index
) {
    if (dissector->tsval == 0)
        return;

    //bpf_debug("[FLOWS][%d] TSVAL: %u, TSECR: %u", direction, tsval, tsecr);

    // Update TSval in forward (rate_index) direction
    if (
        data->tsval[rate_index] == 0 || // No previous TSval
        u32wrap_lt(data->tsval[rate_index], dissector->tsval) // New TSval
    ) {
        data->tsval[rate_index] = dissector->tsval;

        // Only attempt to track TSval if it's not a pure ACK
        if (get_tcp_segment_size(dissector) > 0 || BITCHECK(DIS_TCP_SYN))
            record_tsval(&data->tsval_tstamps[rate_index], dissector->now,
                         dissector->tsval);
    }

    if (dissector->tsecr == 0)
        return;

    // Update TSecr in forward direction + check match in reverse (other_rate_index) direction
    if (
        data->tsecr[rate_index] == 0 || // No previous TSecr
        u32wrap_lt(data->tsecr[rate_index], dissector->tsecr) // New TSecr
    ) {
        data->tsecr[rate_index] = dissector->tsecr;

        // Match TSecr against previous TSval in reverse direction
        __u64 match_at = match_and_clear_recorded_tsval(
            &data->tsval_tstamps[other_rate_index], dissector->tsecr);
        if (match_at > 0) {
            __u64 elapsed = dissector->now - match_at;

            if (data->last_rtt[other_rate_index] + MIN_RTT_SAMPLE_INTERVAL < dissector->now) {
                struct flowbee_event event = {0};
                event.key = *key;
                event.round_trip_time = elapsed;
                event.effective_direction = other_rate_index; // direction of the origial TCP segment we matched against
                bpf_ringbuf_output(&flowbee_events, &event, sizeof(event), 0);
                data->last_rtt[other_rate_index] = dissector->now;
            }
        }
    }

    return;
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
        __u32 zero = 0;
        struct flow_data_t *data = bpf_map_lookup_elem(&flowbee_scratch, &zero);
        if (!data) {
            bpf_debug("[FLOWS] Failed to allocate scratch flow");
            return;
        }
        init_flow_data(dissector, data);
        data->ip_flags = 0; // Obtain these
        if (bpf_map_update_elem(&flowbee, &key, data, BPF_ANY) != 0) {
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

    // Check TCP timestamps and attempt to calculate RTT
    infer_tcp_rtt(dissector, &key, data, rate_index, other_rate_index);

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
    // Ensure that we get DownUp order in the lqosd map
    if (direction == TO_INTERNET) {
        rate_index = 1;
        other_rate_index = 0;
    } else {
        rate_index = 0;
        other_rate_index = 1;
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
