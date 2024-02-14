// TCP flow monitor system

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include "dissector.h"
#include "debug.h"

// Defines a TCP connection flow key
struct tcp_flow_key_t {
    struct in6_addr src;
    struct in6_addr dst;
    __u16 src_port;
    __u16 dst_port;
};

// TCP connection flow entry
struct tcp_flow_data_t {
    __u64 last_seen_a;
    __u64 last_seen_b;
    __u64 bytes_sent;
    __u64 bytes_received;
    __u32 time_a;
    __u32 time_b;
    __u64 last_rtt;
    __u64 packets_sent;
    __u64 packets_received;
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

static __always_inline struct tcp_flow_key_t build_flow_key(
    struct dissector_t *dissector, // The packet dissector from the previous step
    u_int8_t direction // The direction of the packet (1 = to internet, 2 = to local network)
) {
    if (direction == 1) {
        return (struct tcp_flow_key_t) {
            .src = dissector->src_ip,
            .dst = dissector->dst_ip,
            .src_port = dissector->src_port,
            .dst_port = dissector->dst_port
        };
    } else {
        return (struct tcp_flow_key_t) {
            .src = dissector->dst_ip,
            .dst = dissector->src_ip,
            .src_port = dissector->dst_port,
            .dst_port = dissector->src_port
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
    
    // This 8 should be 10, but we're running out of space
    for (u_int8_t i = 0; i<8; i++) {
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

static __always_inline void track_flows(
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
            .last_seen_a = now,
            .last_seen_b = now,
            .bytes_sent = dissector->skb_len,
            .bytes_received = 0,
            .time_a = 0,
            .time_b = 0,
            .last_rtt = 0,
            .packets_sent = 1,
            .packets_received = 0
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
    __u32 tsval = 0;
    __u32 tsecr = 0;
    void *end_opts = (tcp + 1) + (tcp->doff << 2);
    bool has_data = end_opts - dissector->start < dissector->skb_len;
    get_timestamps(&tsval, &tsecr, tcp, dissector, end_opts);

    if ( tcp->ack && has_data) {
        //bpf_debug("Direction %d", direction);
        //bpf_debug("to 192.168.66.%d => SEQ %d <-> %d", dissector->dst_ip.in6_u.u6_addr8[15], bpf_ntohs(tcp->seq), bpf_ntohs(tcp->ack_seq));
        __u32 sequence = bpf_ntohl(tcp->seq);

        if (direction == 1) {
            // Going TO the Internet. We're acknowledging a packet.
            // We don't need to record an RTT measurement and check for issues.
            //bpf_debug("%d / %d", data->time_a, data->time_b);

            if (data->time_a != 0 && sequence < data->time_a) {
                // This is a retransmission
                bpf_debug("DIR 1 Retransmission (or out of order) detected");
                bpf_debug("to 192.168.66.%d => SEQ %d < %d", dissector->dst_ip.in6_u.u6_addr8[15], sequence, data->time_a);
            }

            data->time_a = sequence;
        } else {
            // Coming FROM the Internet. They are acknowledging a packet.
            // We need to record an RTT measurement, but we can check for issues.
            //bpf_debug("%d / %d", data->time_a, data->time_b);

            __u64 rtt = now - last_seen;
            //bpf_debug("RTT: %d nanos", rtt);
            data->last_rtt = rtt;

            if (data->time_b != 0 && sequence < data->time_b) {
                // This is a retransmission
                bpf_debug("DIR 2 Retransmission (or out of order) detected");
                bpf_debug("to 192.168.66.%d => SEQ %d > %d", dissector->dst_ip.in6_u.u6_addr8[15], sequence, data->time_b);
            }

            data->time_b = sequence;
        }


        //bpf_debug("to 192.168.66.%d => TS  %d <-> %d", dissector->dst_ip.in6_u.u6_addr8[15], bpf_ntohs(tsval), bpf_ntohs(tsecr));
    } else if ( tcp->fin) {
        // FIN packet. We are closing a connection.
        // We need to remove this flow from the tracking table.
        bpf_debug("TCP connection closed");
        // TODO: Submit the result somewhere
        bpf_debug(" BYTES   : %d / %d", data->bytes_sent, data->bytes_received);
        bpf_debug(" PACKETS : %d / %d", data->packets_sent, data->packets_received);
        bpf_debug(" RTT     : %d", data->last_rtt);
        // /TODO
        bpf_map_delete_elem(&flowbee, &key);
    } else if ( tcp->rst ) {
        // RST packet. We are resetting a connection.
        // We need to remove this flow from the tracking table.
        bpf_debug("TCP connection reset");
        // TODO: Submit the result somewhere
        bpf_debug(" BYTES   : %d / %d", data->bytes_sent, data->bytes_received);
        bpf_debug(" PACKETS : %d / %d", data->packets_sent, data->packets_received);
        bpf_debug(" RTT     : %d", data->last_rtt);
        // /TODO
        bpf_map_delete_elem(&flowbee, &key);
    }
}
