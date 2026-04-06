#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_endian.h>
#include <bpf/bpf_helpers.h>
#include <linux/if_ether.h>
#include <linux/in.h>
#include <linux/in6.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include <linux/tcp.h>
#include <linux/udp.h>
#include <linux/icmp.h>
#include "dissector.h"

struct net_device {
    int ifindex;
} __attribute__((preserve_access_index));

struct sk_buff {
    struct net_device *dev;
    __u16 vlan_tci;
    __u32 len;
    unsigned char *data;
} __attribute__((preserve_access_index));

struct kprobe_dissector_t {
    struct sk_buff *skb;
    unsigned char *start;
    unsigned char *end;
    struct ethhdr ethernet_header;
    struct in6_addr src_ip;
    struct in6_addr dst_ip;
    struct iphdr ipv4_header;
    struct ipv6hdr ipv6_header;
    __u64 now;
    __u32 skb_len;
    __u32 l4offset;
    __u16 l3offset;
    __u16 eth_type;
    __be16 current_vlan;
    __u16 src_port;
    __u16 dst_port;
    __u16 window;
    __u32 tsval;
    __u32 tsecr;
    __u32 sequence;
    __u8 ip_protocol;
    __u8 tos;
    __u8 tcp_flags;
};

static __always_inline int kprobe_parse_tcp_ts(
    struct kprobe_dissector_t *dissector,
    struct tcphdr *tcph
)
{
    int len = tcph->doff << 2;
    __u32 opt_offset = dissector->l4offset + sizeof(struct tcphdr);
    __u32 opt_end = dissector->l4offset + len;
    __u8 i;

    if (len <= sizeof(struct tcphdr) || opt_end > dissector->skb_len) {
        return -1;
    }

#pragma unroll
    for (i = 0; i < MAX_TCP_OPTIONS; i++) {
        __u8 opt = 0;
        __u8 opt_size = 0;
        if (opt_offset + 1 > opt_end) {
            return -1;
        }
        if (bpf_probe_read_kernel(&opt, sizeof(opt), dissector->start + opt_offset) < 0) {
            return -1;
        }
        if (opt == 0) {
            return -1;
        }
        if (opt == 1) {
            opt_offset++;
            continue;
        }

        if (opt_offset + 2 > opt_end) {
            return -1;
        }
        if (bpf_probe_read_kernel(&opt_size, sizeof(opt_size), dissector->start + opt_offset + 1) < 0) {
            return -1;
        }
        if (opt_size < 2) {
            return -1;
        }

        if (opt == 8 && opt_size == 10) {
            __u32 ts_data[2] = {0};
            if (opt_offset + 10 > opt_end) {
                return -1;
            }
            if (bpf_probe_read_kernel(&ts_data, sizeof(ts_data), dissector->start + opt_offset + 2) < 0) {
                return -1;
            }
            dissector->tsval = bpf_ntohl(ts_data[0]);
            dissector->tsecr = bpf_ntohl(ts_data[1]);
            return 0;
        }

        opt_offset += opt_size;
    }
    return -1;
}

static __always_inline void kprobe_snoop(struct kprobe_dissector_t *dissector)
{
    switch (dissector->ip_protocol) {
    case IPPROTO_TCP:
    {
        struct tcphdr hdr = {0};
        if (dissector->l4offset + sizeof(struct tcphdr) > dissector->skb_len) {
            return;
        }
        if (bpf_probe_read_kernel(&hdr, sizeof(hdr), dissector->start + dissector->l4offset) < 0) {
            return;
        }
        dissector->src_port = hdr.source;
        dissector->dst_port = hdr.dest;
        dissector->window = hdr.window;
        dissector->sequence = hdr.seq;
        if (hdr.fin) dissector->tcp_flags |= DIS_TCP_FIN;
        if (hdr.syn) dissector->tcp_flags |= DIS_TCP_SYN;
        if (hdr.rst) dissector->tcp_flags |= DIS_TCP_RST;
        if (hdr.psh) dissector->tcp_flags |= DIS_TCP_PSH;
        if (hdr.ack) dissector->tcp_flags |= DIS_TCP_ACK;
        if (hdr.urg) dissector->tcp_flags |= DIS_TCP_URG;
        if (hdr.ece) dissector->tcp_flags |= DIS_TCP_ECE;
        if (hdr.cwr) dissector->tcp_flags |= DIS_TCP_CWR;
        kprobe_parse_tcp_ts(dissector, &hdr);
    } break;
    case IPPROTO_UDP:
    {
        struct udphdr hdr = {0};
        if (dissector->l4offset + sizeof(struct udphdr) > dissector->skb_len) {
            return;
        }
        if (bpf_probe_read_kernel(&hdr, sizeof(hdr), dissector->start + dissector->l4offset) < 0) {
            return;
        }
        dissector->src_port = hdr.source;
        dissector->dst_port = hdr.dest;
    } break;
    case IPPROTO_ICMP:
    {
        struct icmphdr hdr = {0};
        if (dissector->l4offset + sizeof(struct icmphdr) > dissector->skb_len) {
            return;
        }
        if (bpf_probe_read_kernel(&hdr, sizeof(hdr), dissector->start + dissector->l4offset) < 0) {
            return;
        }
        dissector->ip_protocol = 1;
        dissector->src_port = bpf_ntohs(hdr.type);
        dissector->dst_port = bpf_ntohs(hdr.code);
    } break;
    }
}

static __always_inline bool kprobe_dissector_new(
    struct sk_buff *skb,
    struct kprobe_dissector_t *dissector
)
{
    __builtin_memset(dissector, 0, sizeof(*dissector));
    dissector->skb = skb;
    dissector->start = BPF_CORE_READ(skb, data);
    dissector->skb_len = BPF_CORE_READ(skb, len);
    dissector->end = dissector->start + dissector->skb_len;
    dissector->current_vlan = bpf_htons(BPF_CORE_READ(skb, vlan_tci));
    dissector->now = bpf_ktime_get_boot_ns();

    if (!dissector->start || dissector->skb_len < sizeof(struct ethhdr)) {
        return false;
    }
    if (bpf_probe_read_kernel(
            &dissector->ethernet_header,
            sizeof(dissector->ethernet_header),
            dissector->start
        ) < 0) {
        return false;
    }

    return true;
}

static __always_inline bool kprobe_find_current_vlan(
    struct kprobe_dissector_t *dissector,
    __be16 *current_vlan
)
{
    *current_vlan = dissector->current_vlan;
    if (*current_vlan != 0) {
        return true;
    }
    return false;
}

static __always_inline bool kprobe_dissector_find_l3_offset(
    struct kprobe_dissector_t *dissector
)
{
    __u32 offset = sizeof(struct ethhdr);
    __u16 eth_type = bpf_ntohs(dissector->ethernet_header.h_proto);

    if (eth_type == ETH_P_IP || eth_type == ETH_P_IPV6) {
        dissector->eth_type = eth_type;
        dissector->l3offset = offset;
        return true;
    }

    if (eth_type == ETH_P_ARP || eth_type < ETH_P_802_3_MIN || eth_type == 0xFEFE) {
        return false;
    }

    __u8 i = 0;
    while (i < 10 && !is_ip(eth_type)) {
        switch (eth_type) {
        case ETH_P_8021AD:
        case ETH_P_8021Q:
        {
            struct vlan_hdr vlan = {0};
            if (offset + sizeof(struct vlan_hdr) > dissector->skb_len) {
                return false;
            }
            if (bpf_probe_read_kernel(&vlan, sizeof(vlan), dissector->start + offset) < 0) {
                return false;
            }
            dissector->current_vlan = vlan.h_vlan_TCI;
            eth_type = bpf_ntohs(vlan.h_vlan_encapsulated_proto);
            offset += sizeof(struct vlan_hdr);
        }
        break;
        case ETH_P_PPP_SES:
        {
            struct pppoe_proto pppoe = {0};
            __u16 proto;
            if (offset + sizeof(struct pppoe_proto) > dissector->skb_len) {
                return false;
            }
            if (bpf_probe_read_kernel(&pppoe, sizeof(pppoe), dissector->start + offset) < 0) {
                return false;
            }
            proto = bpf_ntohs(pppoe.proto);
            switch (proto) {
            case PPP_IP:
                eth_type = ETH_P_IP;
                break;
            case PPP_IPV6:
                eth_type = ETH_P_IPV6;
                break;
            default:
                return false;
            }
            offset += PPPOE_SES_HLEN;
        }
        break;
        case ETH_P_MPLS_UC:
        case ETH_P_MPLS_MC:
        {
            struct mpls_label mpls = {0};
            struct iphdr iph = {0};
            if (offset + sizeof(struct mpls_label) > dissector->skb_len) {
                return false;
            }
            if (bpf_probe_read_kernel(&mpls, sizeof(mpls), dissector->start + offset) < 0) {
                return false;
            }
            offset += 4;
            if (mpls.entry & MPLS_LS_S_MASK) {
                if (offset + sizeof(struct iphdr) > dissector->skb_len) {
                    return false;
                }
                if (bpf_probe_read_kernel(&iph, sizeof(iph), dissector->start + offset) < 0) {
                    return false;
                }
                switch (iph.version) {
                case 4:
                    eth_type = ETH_P_IP;
                    break;
                case 6:
                    eth_type = ETH_P_IPV6;
                    break;
                default:
                    return false;
                }
            }
        }
        break;
        default:
            return false;
        }
        ++i;
    }

    dissector->l3offset = offset;
    dissector->eth_type = eth_type;
    return true;
}

static __always_inline bool kprobe_dissector_find_ip_header(
    struct kprobe_dissector_t *dissector
)
{
    switch (dissector->eth_type) {
    case ETH_P_IP:
    {
        if (dissector->l3offset + sizeof(struct iphdr) > dissector->skb_len) {
            return false;
        }
        if (bpf_probe_read_kernel(
                &dissector->ipv4_header,
                sizeof(dissector->ipv4_header),
                dissector->start + dissector->l3offset
            ) < 0) {
            return false;
        }
        encode_ipv4(dissector->ipv4_header.saddr, &dissector->src_ip);
        encode_ipv4(dissector->ipv4_header.daddr, &dissector->dst_ip);
        dissector->ip_protocol = dissector->ipv4_header.protocol;
        dissector->tos = dissector->ipv4_header.tos;
        dissector->l4offset = dissector->l3offset + (dissector->ipv4_header.ihl * 4);
        kprobe_snoop(dissector);
        return true;
    }
    break;
    case ETH_P_IPV6:
    {
        if (dissector->l3offset + sizeof(struct ipv6hdr) > dissector->skb_len) {
            return false;
        }
        if (bpf_probe_read_kernel(
                &dissector->ipv6_header,
                sizeof(dissector->ipv6_header),
                dissector->start + dissector->l3offset
            ) < 0) {
            return false;
        }
        encode_ipv6(&dissector->ipv6_header.saddr, &dissector->src_ip);
        encode_ipv6(&dissector->ipv6_header.daddr, &dissector->dst_ip);
        dissector->ip_protocol = dissector->ipv6_header.nexthdr;
        dissector->tos = dissector->ipv6_header.flow_lbl[0];
        dissector->l4offset = dissector->l3offset + sizeof(struct ipv6hdr);
        kprobe_snoop(dissector);
        return true;
    }
    break;
    default:
        return false;
    }
}
