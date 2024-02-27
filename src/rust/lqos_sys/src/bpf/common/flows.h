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
struct tcp_flow_key_t {
    struct in6_addr src;
    struct in6_addr dst;
    __u16 src_port;
    __u16 dst_port;
};

// TCP connection flow entry
struct tcp_flow_data_t {
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
    __u64 rate_estimate[2];
    // Sequence number of the last packet
    __u32 last_sequence[2];
    // Acknowledgement number of the last packet
    __u32 last_ack[2];
    // Retry Counters
    __u32 retries[2];
    // Timestamp values
    __u32 tsval[2];
    __u32 tsecr[2];
    __u64 ts_change_time[2];
    __u64 ts_calc_time[2];
    // Most recent RTT
    __u64 last_rtt[2];
};

// Map for tracking TCP flow progress.
// This is pinned and not per-CPU, because half the data appears on either side of the bridge.
struct
{
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __type(key, struct tcp_flow_key_t);
    __type(value, struct tcp_flow_data_t);
    __uint(max_entries, MAX_FLOWS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} flowbee SEC(".maps");

static __always_inline struct tcp_flow_key_t build_tcp_flow_key(
    struct dissector_t *dissector, // The packet dissector from the previous step
    struct tcphdr *tcp, // The TCP header
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    if (direction == FROM_INTERNET) {
        return (struct tcp_flow_key_t) {
            .src = dissector->src_ip,
            .dst = dissector->dst_ip,
            .src_port = tcp->source,
            .dst_port = tcp->dest,
        };
    } else {
        return (struct tcp_flow_key_t) {
            .src = dissector->dst_ip,
            .dst = dissector->src_ip,
            .src_port = tcp->dest,
            .dst_port = tcp->source,
        };    
    }
}

static __always_inline void debug_ip(
    struct in6_addr *ip
) {
    bpf_debug("%d.%d.%d", ip->s6_addr[13], ip->s6_addr[14], ip->s6_addr[15]);
}

static __always_inline bool get_timestamps(
    u_int32_t * out_tsval,
    u_int32_t * out_tsecr,
    struct tcphdr * tcp,
    struct dissector_t * dissector,
    void * end_opts  
) {
    u_int8_t *pos = (u_int8_t *)(tcp + 1); // Current pos in TCP options
    u_int8_t len;
    
    // This should be 10, but we're running out of space
    for (u_int8_t i = 0; i<6; i++) {
        if (pos + 2 > dissector->end) {
            return false;
        }    
        switch (*pos) {
            case 0: return false; // End of options
            case 1: pos++; break; // NOP
            case 8: {
                if (pos + 10 > dissector->end) {
                    return false;
                }
                *out_tsval = bpf_ntohl(*(__u32 *)(pos + 2));
                *out_tsecr = bpf_ntohl(*(__u32 *)(pos + 6));
                return true;
            }
            default: {
                len = *(pos + 1);
                pos += len;
            }
        }
    }

    return false;
}

// Handle Per-Flow ICMP Analysis
static __always_inline void process_icmp(
    struct dissector_t *dissector,
    u_int8_t direction,
    struct icmphdr *icmp
) {

}

// Handle Per-Flow UDP Analysis
static __always_inline void process_udp(
    struct dissector_t *dissector,
    u_int8_t direction,
    struct udphdr *udp
) {
    
}

// Handle Per-Flow TCP Analysis
static __always_inline void process_tcp(
    struct dissector_t *dissector,
    u_int8_t direction,
    struct tcphdr *tcp,
    u_int64_t now
) {
    if ((tcp->syn && !tcp->ack && direction == TO_INTERNET) || (tcp->syn && tcp->ack && direction == FROM_INTERNET)) {
        // A customer is requesting a new TCP connection. That means
        // we need to start tracking this flow.
        bpf_debug("[FLOWS] New TCP Connection Detected (%u)", direction);
        struct tcp_flow_key_t key = build_tcp_flow_key(dissector, tcp, direction);
        struct tcp_flow_data_t data = {
            .start_time = now,
            .bytes_sent = { 0, 0 },
            .packets_sent = { 0, 0 },
            .next_count_time = { now + SECOND_IN_NANOS, now + SECOND_IN_NANOS },
            .last_count_time = { now, now },
            .next_count_bytes = { dissector->skb_len, dissector->skb_len },
            .rate_estimate = { 0, 0 },
            .last_sequence = { 0, 0 },
            .last_ack = { 0, 0 },
            .retries = { 0, 0 },
            .tsval = { 0, 0 },
            .tsecr = { 0, 0 },
            .ts_change_time = { 0, 0 },
            .ts_calc_time = { now + TIMESTAMP_INTERVAL_NANOS, now + TIMESTAMP_INTERVAL_NANOS },
            .last_rtt = { 0, 0 }
        };
        if (bpf_map_update_elem(&flowbee, &key, &data, BPF_ANY) != 0) {
            bpf_debug("[FLOWS] Failed to add new flow to map");
        }
        return;
    }

    // Build the flow key
    struct tcp_flow_key_t key = build_tcp_flow_key(dissector, tcp, direction);
    struct tcp_flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        // If it isn't a flow we're tracking, bail out now
        return;
    }

    // Update last seen to now
    data->last_seen = now;

    // Update bytes and packets sent
    if (direction == TO_INTERNET) {
        data->bytes_sent[0] += dissector->skb_len;
        data->packets_sent[0]++;

        if (now > data->next_count_time[0]) {
            // Calculate the rate estimate
            __u64 bits = (data->bytes_sent[0] - data->next_count_bytes[0])*8;
            __u64 time = (now - data->last_count_time[0]) / 1000000000; // Seconds
            data->rate_estimate[0] = bits/time;
            //bpf_debug("[FLOWS][%d] Rate Estimate: %u mbits / second", direction, data->rate_estimate[0] / 1000000);
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
            data->rate_estimate[1] = bits/time;
            //bpf_debug("[FLOWS][%d] Rate Estimate: %u mbits / second", direction, data->rate_estimate[1] / 1000000);
            data->next_count_time[1] = now + SECOND_IN_NANOS;
            data->next_count_bytes[1] = data->bytes_sent[1];
            data->last_count_time[1] = now;
        }
    }

    // Sequence and Acknowledgement numbers
    __u32 sequence = bpf_ntohl(tcp->seq);
    __u32 ack_seq = bpf_ntohl(tcp->ack_seq);
    if (direction == TO_INTERNET) {
        if (data->last_sequence[0] != 0 && sequence < data->last_sequence[0]) {
            // This is a retransmission
            //bpf_debug("[FLOWS] Retransmission detected (%u)", direction);
            data->retries[0]++;
        }

        data->last_sequence[0] = sequence;
        data->last_ack[0] = ack_seq;
    } else {
        if (data->last_sequence[1] != 0 && sequence < data->last_sequence[1]) {
            // This is a retransmission
            //bpf_debug("[FLOWS] Retransmission detected (%u)", direction);
            data->retries[1]++;
        }

        data->last_sequence[1] = sequence;
        data->last_ack[1] = ack_seq;        
    }
    //bpf_debug("[FLOWS][%d] Sequence: %u Ack: %u", direction, sequence, ack_seq);

    // Timestamps to calculate RTT
    u_int32_t tsval = 0;
    u_int32_t tsecr = 0;
    void *end_opts = (tcp + 1) + (tcp->doff << 2);
    if (tcp->ack && get_timestamps(&tsval, &tsecr, tcp, dissector, end_opts)) {
        //bpf_debug("[FLOWS][%d] TSVal %u TSecr %u", direction, tsval, tsecr);
        if (direction == TO_INTERNET) {
            if (tsval != data->tsval[0] || tsecr != data->tsecr[0]) {

                if (tsval == data->tsecr[1]) {
                    //bpf_debug("%d Matched!", direction);
                    __u64 elapsed = now - data->ts_change_time[1];
                    data->last_rtt[0] = elapsed;
                    //bpf_debug("%d TS Change (RTT): %u nanos", direction, elapsed);
                    // TODO: Do something with the RTT
                }

                //bpf_debug("%d TSVal Changed", direction);
                data->ts_change_time[0] = now;
                data->tsval[0] = tsval;
                data->tsecr[0] = tsecr;
            }
        } else {
            if (tsval != data->tsval[1] || tsecr != data->tsecr[1]) {

                if (tsval == data->tsecr[0]) {
                    //bpf_debug("%d Matched!", direction);
                    __u64 elapsed = now - data->ts_change_time[0];
                    data->last_rtt[1] = elapsed;
                    //bpf_debug("%d TS Change (RTT): %u nanos", direction, elapsed);
                    // TODO: Do something with the RTT
                }

                //bpf_debug("%d TSVal Changed", direction);
                data->ts_change_time[1] = now;
                data->tsval[1] = tsval;
                data->tsecr[1] = tsecr;
            }
        }


        /*else {
            if (tsval == data->tsecr[0]) {
            //if (tsval == data->tsecr[0] && now > data->ts_calc_time[1]) {
                __u64 elapsed = now - data->ts_change_time[0];
                bpf_debug("[FLOWS][%d] TS Change (RTT): %u nanos", direction, elapsed);
                data->ts_calc_time[1] = now + TIMESTAMP_INTERVAL_NANOS;
            }
            if (tsval != data->tsval[1]) {
                data->ts_change_time[1] = now;
            }
            data->tsval[1] = tsval;
            data->tsecr[1] = tsecr;
        }*/
    }

    // Has the connection ended?
    if (tcp->fin || tcp->rst) {
        __u64 lifetime = now - data->start_time;
        bpf_debug("[FLOWS] TCP Connection Ended [%d / %d]. Lasted %u nanos.", data->bytes_sent[0], data->bytes_sent[1], lifetime);
        bpf_debug("[FLOWS] Rate Estimate (Mbps): %u / %u", data->rate_estimate[0] / 1000000, data->rate_estimate[1] / 1000000);
        bpf_debug("[FLOWS] Retries: %u / %u", data->retries[0], data->retries[1]);
        bpf_debug("[FLOWS] RTT: %u / %u (nanos)", data->last_rtt[0], data->last_rtt[1]);
        bpf_map_delete_elem(&flowbee, &key);
    }
}

// Note that this duplicates a lot of what we do for "snoop" - we're hoping
// to replace both it and the old RTT system.
static __always_inline void track_flows(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    //bpf_debug("[FLOWS] Packet detected");
    __u64 now = bpf_ktime_get_ns();
    switch (dissector->ip_protocol)
    {
        case IPPROTO_TCP: {
            struct tcphdr * tcp = get_tcp_header(dissector);
            if (tcp == NULL) {
                // Bail out if it's not a TCP packet
                return;
            }
            // Bail out if we've exceeded the packet size and there is no payload
            // This keeps the safety checker happy and is generally a good idea
            if (tcp + 1 >= dissector->end) {
                return;
            }
            //bpf_debug("[FLOWS] TCP packet detected");
            process_tcp(dissector, direction, tcp, now);
        } break;
        case IPPROTO_UDP: {
            struct udphdr *udp = get_udp_header(dissector);
            if (udp == NULL) {
                // Bail out if it's not a UDP packet
                return;
            }
            // Bail out if we've exceeded the packet size and there is no payload
            // This keeps the safety checker happy and is generally a good idea
            if (udp + 1 >= dissector->end) {
                return;
            }
            bpf_debug("[FLOWS] UDP packet detected");
            process_udp(dissector, direction, udp);
        } break;
        case IPPROTO_ICMP: {
            struct icmphdr *icmp = get_icmp_header(dissector);
            if (icmp == NULL) {
                // Bail out if it's not an ICMP packet
                return;
            }
            // Bail out if we've exceeded the packet size and there is no payload
            // This keeps the safety checker happy and is generally a good idea
            if (icmp + 1 >= dissector->end) {
                return;
            }
            bpf_debug("[FLOWS] ICMP packet detected");
            process_icmp(dissector, direction, icmp);
        } break;
        default: {
            bpf_debug("[FLOWS] Unsupported protocol: %d", dissector->ip_protocol);
        }
    }
}

/*static __always_inline void track_flows(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    struct tcphdr * tcp = get_tcp_header(dissector);
    if (tcp == NULL) {
        // Bail out if it's not a TCP packet
        return;
    }

    // Bail out if we've exceeded the packet size and there is no payload
    // This keeps the safety checker happy and is generally a good idea
    if (tcp + 1 >= dissector->end) {
        return;
    }

    // Determine the key for the flow. Since we know direction, there's
    // no need to consider "reverse keys" and their ilk.
    struct tcp_flow_key_t key = build_flow_key(dissector, direction);

    // Only care about connections that originate locally
    __u64 now = bpf_ktime_get_ns();
    if (tcp->syn && direction == 1) {
        // SYN packet sent to the Internet. We are establishing a new connection.
        // We need to add this flow to the tracking table.
        bpf_debug("New TCP connection detected");
        struct tcp_flow_data_t data = {
            .start_time = now,
            .last_seen_a = now,
            .last_seen_b = now,
            .bytes_sent = dissector->skb_len,
            .bytes_received = 0,
            .time_a = 0,
            .time_b = 0,
            .last_rtt = 0,
            .packets_sent = 1,
            .packets_received = 0,
            .retries_a = 0,
            .retries_b = 0,
            .next_count_time = now + SECOND_IN_NANOS,
            .next_count_bytes = dissector->skb_len,
            .rate_estimate = 0,
            .last_count_time = now
        };
        bpf_map_update_elem(&flowbee, &key, &data, BPF_ANY);
    }

    // Update the flow's last seen time
    struct tcp_flow_data_t *data = bpf_map_lookup_elem(&flowbee, &key);
    if (data == NULL) {
        return;
    }
    __u64 last_seen = data->last_seen_a;
    if (direction == 1) {
        data->last_seen_a = now;
        data->bytes_sent += dissector->skb_len;
        data->packets_sent++;
    } else {
        data->last_seen_b = now;
        data->bytes_received += dissector->skb_len;
        data->packets_received++;
    }
    //bpf_debug("Dir: %d, Sent/Received: [%d]/[%d]", direction, data->bytes_sent, data->bytes_received);

    // Parse the TCP options
    //__u32 tsval = 0;
    //__u32 tsecr = 0;
    void *end_opts = (tcp + 1) + (tcp->doff << 2);
    bool has_data = end_opts - dissector->start < dissector->skb_len;
    //if (get_timestamps(&tsval, &tsecr, tcp, dissector, end_opts)) {
        //bpf_debug("[%d] => TSVal %u TSecr %u", direction, tsval, tsecr);
        //bpf_debug("[%d] => Seq %u AckSeq %u", direction, tcp->seq, tcp->ack_seq);
    //}

    if ( tcp->ack && has_data) {
        //bpf_debug("Direction %d", direction);        
        __u32 sequence = bpf_ntohl(tcp->seq);
        __u32 ack_seq = bpf_ntohl(tcp->ack_seq);

        if (direction == 1) {
            // Going TO the Internet. We're acknowledging a packet.
            // We don't need to record an RTT measurement and check for issues.
            bpf_debug("%u, A: %u / B: %u", sequence, data->time_a, data->time_b);
            bpf_debug("%u", ack_seq);

            if (now > data->next_count_time) {
                // Calculate the rate estimate
                __u64 bytes = data->bytes_sent + data->bytes_received - data->next_count_bytes;
                __u64 time = now - data->last_count_time;
                data->rate_estimate = ((bytes * SECOND_IN_NANOS / time)*8)/1048576;
                data->next_count_time = now + SECOND_IN_NANOS;
                data->next_count_bytes = data->bytes_sent + data->bytes_received;
                data->last_count_time = now;
                bpf_debug("[1] Rate estimate: %u mbits/sec", data->rate_estimate);

                if (data->rate_estimate > 5 && tcp->ack_seq >= data->time_a) {
                    __u64 rtt = now - last_seen;
                    bpf_debug("RTT: %d nanos (%u - %u)", rtt, tcp->ack_seq, data->time_a);
                    data->last_rtt = rtt;
                }
            }

            if (data->rate_estimate > 5 && ack_seq >= data->time_b) {
                    __u64 rtt = now - last_seen;
                    bpf_debug("[1] RTT: %d nanos (%u - %u)", rtt, sequence, data->time_b);
                    data->last_rtt = rtt;
                }

            if (data->time_a != 0 && sequence < data->time_a) {
                // This is a retransmission
                //bpf_debug("DIR 1 Retransmission (or out of order) detected");
                //bpf_debug("to 192.168.66.%d => SEQ %d < %d", dissector->dst_ip.in6_u.u6_addr8[15], sequence, data->time_a);
                data->retries_a++;
            }

            data->time_a = sequence;
        } else {
            // Coming FROM the Internet. They are acknowledging a packet.
            // We need to record an RTT measurement, but we can check for issues.
            //bpf_debug("%d / %d", data->time_a, data->time_b);

            if (now > data->next_count_time) {
                // Calculate the rate estimate
                __u64 bytes = data->bytes_sent + data->bytes_received - data->next_count_bytes;
                __u64 time = now - data->last_count_time;
                data->rate_estimate = ((bytes * SECOND_IN_NANOS / time)*8)/1000000;
                data->next_count_time = now + SECOND_IN_NANOS;
                data->next_count_bytes = data->bytes_sent + data->bytes_received;
                data->last_count_time = now;
                bpf_debug("[2] Rate estimate: %u mbits/sec", data->rate_estimate);

                if (data->rate_estimate > 5 && tcp->ack_seq >= data->time_b) {
                    __u64 rtt = now - last_seen;
                    bpf_debug("[2] RTT: %d nanos", rtt);
                    data->last_rtt = rtt;
                }
            }


            if (data->time_b != 0 && sequence < data->time_b) {
                // This is a retransmission
                //bpf_debug("DIR 2 Retransmission (or out of order) detected");
                //bpf_debug("to 192.168.66.%d => SEQ %d > %d", dissector->dst_ip.in6_u.u6_addr8[15], sequence, data->time_b);
                data->retries_b++;
            }

            data->time_b = sequence;
        }


        //bpf_debug("to 192.168.66.%d => TS  %d <-> %d", dissector->dst_ip.in6_u.u6_addr8[15], bpf_ntohs(tsval), bpf_ntohs(tsecr));
    } else if ( tcp->fin) {
        // FIN packet. We are closing a connection.
        // We need to remove this flow from the tracking table.
        bpf_debug("TCP connection closed");
        // TODO: Submit the result somewhere
        bpf_debug(" Flow Lifetime: %u nanos", now - data->start_time);
        bpf_debug(" BYTES   : %d / %d", data->bytes_sent, data->bytes_received);
        bpf_debug(" PACKETS : %d / %d", data->packets_sent, data->packets_received);
        bpf_debug(" RTT     : %d nanos", data->last_rtt);
        bpf_debug(" RETRIES : %d / %d", data->retries_a, data->retries_b);
        // /TODO
        bpf_map_delete_elem(&flowbee, &key);
    } else if ( tcp->rst ) {
        // RST packet. We are resetting a connection.
        // We need to remove this flow from the tracking table.
        bpf_debug("TCP connection reset");
        // TODO: Submit the result somewhere
        bpf_debug(" Flow Lifetime: %u nanos", now - data->start_time);
        bpf_debug(" BYTES   : %d / %d", data->bytes_sent, data->bytes_received);
        bpf_debug(" PACKETS : %d / %d", data->packets_sent, data->packets_received);
        bpf_debug(" RTT     : %d nanos", data->last_rtt);
        bpf_debug(" RETRIES : %d / %d", data->retries_a, data->retries_b);
        // /TODO
        bpf_map_delete_elem(&flowbee, &key);
    }
}*/
