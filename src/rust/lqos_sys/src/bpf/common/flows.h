// TCP flow monitor system

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include "dissector.h"
#include "debug.h"


#define SECOND_IN_NANOS 1000000000
#define TIMESTAMP_INTERVAL_NANOS 2000000000

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
    __u64 rate_estimate_bps[2];
    // Sequence number of the last packet
    __u32 last_sequence[2];
    // Acknowledgement number of the last packet
    __u32 last_ack[2];
    // Retry Counters
    __u32 retries[2];
    // Timestamp values
    __u32 tsval[2];
    __u32 tsecr[2];
    // When did the timestamp change?
    __u64 ts_change_time[2];
    // When should we calculate RTT (to avoid flooding)
    __u64 ts_calc_time[2];
    // Most recent RTT
    __u64 last_rtt[2];
    // Has the connection ended?
    // 0 = Alive, 1 = FIN, 2 = RST
    __u32 end_status;
};

// Map for tracking TCP flow progress.
// This is pinned and not per-CPU, because half the data appears on either side of the bridge.
struct
{
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __type(key, struct flow_key_t);
    __type(value, struct flow_data_t);
    __uint(max_entries, MAX_FLOWS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} flowbee SEC(".maps");

static __always_inline struct flow_data_t new_flow_data(
    __u64 now,
    struct dissector_t *dissector
) {
    struct flow_data_t data = {
        .start_time = now,
        .bytes_sent = { 0, 0 },
        .packets_sent = { 0, 0 },
        .next_count_time = { now + SECOND_IN_NANOS, now + SECOND_IN_NANOS },
        .last_count_time = { now, now },
        .next_count_bytes = { dissector->skb_len, dissector->skb_len },
        .rate_estimate_bps = { 0, 0 },
        .last_sequence = { 0, 0 },
        .last_ack = { 0, 0 },
        .retries = { 0, 0 },
        .tsval = { 0, 0 },
        .tsecr = { 0, 0 },
        .ts_change_time = { 0, 0 },
        .ts_calc_time = { now, now }, // Get a first number quickly
        .last_rtt = { 0, 0 },
        .end_status = 0
    };
    return data;
}

static __always_inline struct flow_key_t build_flow_key(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    if (direction == FROM_INTERNET) {
        return (struct flow_key_t) {
            .src = dissector->src_ip,
            .dst = dissector->dst_ip,
            .src_port = dissector->src_port,
            .dst_port = dissector->dst_port,
            .protocol = dissector->ip_protocol,
            .pad = 0
        };
    } else {
        return (struct flow_key_t) {
            .src = dissector->dst_ip,
            .dst = dissector->src_ip,
            .src_port = dissector->dst_port,
            .dst_port = dissector->src_port,
            .protocol = dissector->ip_protocol,
            .pad = 0
        };    
    }
}

static __always_inline void update_flow_rates(
    struct dissector_t *dissector,
    u_int8_t direction,
    struct flow_data_t *data,
    __u64 now
) {
    data->last_seen = now;

    // Update bytes and packets sent
    if (direction == TO_INTERNET) {
        data->bytes_sent[0] += dissector->skb_len;
        data->packets_sent[0]++;

        if (now > data->next_count_time[0]) {
            // Calculate the rate estimate
            __u64 bits = (data->bytes_sent[0] - data->next_count_bytes[0])*8;
            __u64 time = (now - data->last_count_time[0]) / 1000000000; // Seconds
            data->rate_estimate_bps[0] = bits/time;
            data->next_count_time[0] = now + SECOND_IN_NANOS;
            data->next_count_bytes[0] = data->bytes_sent[0];
            data->last_count_time[0] = now;
        }
    } else {
        data->bytes_sent[1] += dissector->skb_len;
        data->packets_sent[1]++;

        if (now > data->next_count_time[1]) {
            // Calculate the rate estimate
            __u64 bits = (data->bytes_sent[1] - data->next_count_bytes[1])*8;
            __u64 time = (now - data->last_count_time[1]) / 1000000000; // Seconds
            data->rate_estimate_bps[1] = bits/time;
            data->next_count_time[1] = now + SECOND_IN_NANOS;
            data->next_count_bytes[1] = data->bytes_sent[1];
            data->last_count_time[1] = now;
        }
    }
}

// Handle Per-Flow ICMP Analysis
static __always_inline void process_icmp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int64_t now
) {
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // There isn't a flow, so we need to make one
        struct flow_data_t new_data = new_flow_data(now, dissector);
        if (bpf_map_update_elem(&flowbee, &key, &new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        data = bpf_map_lookup_elem(&flowbee, &key);
    }
    update_flow_rates(dissector, direction, data, now);
}

// Handle Per-Flow UDP Analysis
static __always_inline void process_udp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int64_t now
) {
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // There isn't a flow, so we need to make one
        struct flow_data_t new_data = new_flow_data(now, dissector);
        if (bpf_map_update_elem(&flowbee, &key, &new_data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
            return;
        }
        data = bpf_map_lookup_elem(&flowbee, &key);
    }
    update_flow_rates(dissector, direction, data, now);
}

// Handle Per-Flow TCP Analysis
static __always_inline void process_tcp(
    struct dissector_t *dissector,
    u_int8_t direction,
    u_int64_t now
) {
    if ((BITCHECK(DIS_TCP_SYN) && !BITCHECK(DIS_TCP_ACK) && direction == TO_INTERNET) || 
        (BITCHECK(DIS_TCP_SYN) && BITCHECK(DIS_TCP_ACK) && direction == FROM_INTERNET)) {
        // A customer is requesting a new TCP connection. That means
        // we need to start tracking this flow.
        #ifdef VERBOSE
        bpf_debug("[FLOWS] New TCP Connection Detected (%u)", direction);
        #endif
        struct flow_key_t key = build_flow_key(dissector, direction);
        struct flow_data_t data = new_flow_data(now, dissector);
        if (bpf_map_update_elem(&flowbee, &key, &data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
        }
        return;
    }

    // Build the flow key
    struct flow_key_t key = build_flow_key(dissector, direction);
    struct flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // If it isn't a flow we're tracking, bail out now
        return;
    }

    update_flow_rates(dissector, direction, data, now);

    // Sequence and Acknowledgement numbers
    __u32 sequence = bpf_ntohl(dissector->sequence);
    __u32 ack_seq = bpf_ntohl(dissector->ack_seq);
    if (direction == TO_INTERNET) {
        if (data->last_sequence[0] != 0 && sequence < data->last_sequence[0]) {
            // This is a retransmission
            data->retries[0]++;
        }

        data->last_sequence[0] = sequence;
        data->last_ack[0] = ack_seq;
    } else {
        if (data->last_sequence[1] != 0 && sequence < data->last_sequence[1]) {
            // This is a retransmission
            data->retries[1]++;
        }

        data->last_sequence[1] = sequence;
        data->last_ack[1] = ack_seq;        
    }

    // Timestamps to calculate RTT
    u_int32_t tsval = dissector->tsval;
    u_int32_t tsecr = dissector->tsecr;
    if (BITCHECK(DIS_TCP_ACK) && tsval != 0) {
        if (direction == TO_INTERNET) {
            if (tsval != data->tsval[0] || tsecr != data->tsecr[0]) {

                if (tsval == data->tsecr[1]) {
                    if (now > data->ts_calc_time[0]) {
                        __u64 elapsed = now - data->ts_change_time[1];
                        data->ts_calc_time[0] = now + TIMESTAMP_INTERVAL_NANOS;
                        data->last_rtt[0] = elapsed;
                    }
                }

                data->ts_change_time[0] = now;
                data->tsval[0] = tsval;
                data->tsecr[0] = tsecr;
            }
        } else {
            if (tsval != data->tsval[1] || tsecr != data->tsecr[1]) {

                if (tsval == data->tsecr[0]) {
                    if (now > data->ts_calc_time[1]) {
                        __u64 elapsed = now - data->ts_change_time[0];
                        data->ts_calc_time[1] = now + TIMESTAMP_INTERVAL_NANOS;
                        data->last_rtt[1] = elapsed;
                    }
                }

                data->ts_change_time[1] = now;
                data->tsval[1] = tsval;
                data->tsecr[1] = tsecr;
            }
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
    __u64 now = bpf_ktime_get_boot_ns();

    // Pass to the appropriate protocol handler
    switch (dissector->ip_protocol)
    {
        case IPPROTO_TCP: process_tcp(dissector, direction, now); break;
        case IPPROTO_UDP: process_udp(dissector, direction, now); break;
        case IPPROTO_ICMP: process_icmp(dissector, direction, now); break;
        default: {
            #ifdef VERBOSE
            bpf_debug("[FLOWS] Unsupported protocol: %d", dissector->ip_protocol);
            #endif
        }
    }
}
