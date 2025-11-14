#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "../common/skb_safety.h"
#include "../common/debug.h"
#include "../common/ip_hash.h"
#include "../common/bifrost.h"
#include "../common/tcp_opts.h"
#include <linux/in.h>
#include <linux/in6.h>
#include <linux/tcp.h>
#include <linux/udp.h>
#include <linux/icmp.h>

// Packet dissector for XDP. We don't have any help from Linux at this
// point.
struct dissector_t
{
    // Pointer to the XDP context.
    struct xdp_md *ctx;
    // Start of data
    void *start;
    // End of data
    void *end;
    // Ethernet header once found (NULL until then)
    struct ethhdr *ethernet_header;
    // IPv4/6 header once found
    union iph_ptr ip_header;
    // Source IP address, encoded by `ip_hash.h`
    struct in6_addr src_ip;
    // Destination IP address, encoded by `ip_hash.h`
    struct in6_addr dst_ip;
    __u64 now;
    // Total length (end - start)
    __u32 skb_len;
      // Layer-3 offset if found (0 until then)
    __u16 l3offset;
    // Ethernet packet type once found (0 until then)
    __u16 eth_type;
    // Current VLAN tag. If there are multiple tags, it will be
    // the INNER tag.
    __be16 current_vlan;
    __u16 src_port;
    __u16 dst_port;
    __u16 window;
    __u32 tsval;
    __u32 tsecr;
    __u32 sequence;
      // IP protocol from __UAPI_DEF_IN_IPPROTO
    __u8 ip_protocol;
    __u8 tos;
    __u8 tcp_flags;

};

// Representation of the VLAN header type.
struct vlan_hdr
{
    // Tagged VLAN number
    __be16 h_vlan_TCI;
    // Protocol for the next section
    __be16 h_vlan_encapsulated_proto;
};

// Representation of the PPPoE protocol header.
struct pppoe_proto
{
    __u8 pppoe_version_type;
    __u8 ppoe_code;
    __be16 session_id;
    __be16 pppoe_length;
    __be16 proto;
};

#define PPPOE_SES_HLEN 8
#define PPP_IP 0x21
#define PPP_IPV6 0x57

// Representation of an MPLS label
struct mpls_label
{
    __be32 entry;
};

#define MPLS_LS_LABEL_MASK 0xFFFFF000
#define MPLS_LS_LABEL_SHIFT 12
#define MPLS_LS_TC_MASK 0x00000E00
#define MPLS_LS_TC_SHIFT 9
#define MPLS_LS_S_MASK 0x00000100
#define MPLS_LS_S_SHIFT 8
#define MPLS_LS_TTL_MASK 0x000000FF
#define MPLS_LS_TTL_SHIFT 0

// Constructor for a dissector
// Connects XDP/TC SKB structure to a dissector structure.
// Arguments:
// * ctx - an xdp_md structure, passed from the entry-point
// * dissector - pointer to a local dissector object to be initialized
//
// Returns TRUE if all is good, FALSE if the process cannot be completed
static __always_inline bool dissector_new(
    struct xdp_md *ctx,
    struct dissector_t *dissector)
{
    dissector->ctx = ctx;
    dissector->start = (void *)(long)ctx->data;
    dissector->end = (void *)(long)ctx->data_end;
    dissector->ethernet_header = (struct ethhdr *)NULL;
    dissector->l3offset = 0;
    dissector->skb_len = dissector->end - dissector->start;
    dissector->current_vlan = 0;
    dissector->ip_protocol = 0;
    dissector->src_port = 0;
    dissector->dst_port = 0;
    dissector->tos = 0;
    dissector->sequence = 0;
    dissector->now = bpf_ktime_get_boot_ns();

    // Check that there's room for an ethernet header
    if SKB_OVERFLOW (dissector->start, dissector->end, ethhdr)
    {
        return false;
    }
    dissector->ethernet_header = (struct ethhdr *)dissector->start;

    return true;
}

// Helper function - is an eth_type an IPv4 or v6 type?
static __always_inline bool is_ip(__u16 eth_type)
{
    return eth_type == ETH_P_IP || eth_type == ETH_P_IPV6;
}

// Locates the layer-3 offset, if present. Fast returns for various
// common non-IP types. Will perform VLAN redirection if requested.
static __always_inline bool dissector_find_l3_offset(
    struct dissector_t *dissector,
    bool vlan_redirect)
{
    if (dissector->ethernet_header == NULL)
    {
        bpf_debug("Ethernet header is NULL, still called offset check.");
        return false;
    }
    __u32 offset = sizeof(struct ethhdr);
    __u16 eth_type = bpf_ntohs(dissector->ethernet_header->h_proto);

    // Fast return for unwrapped IP
    if (eth_type == ETH_P_IP || eth_type == ETH_P_IPV6)
    {
        dissector->eth_type = eth_type;
        dissector->l3offset = offset;
        return true;
    }

    // Fast return for ARP or non-802.3 ether types (0xFEFE is IS-IS)
    if (eth_type == ETH_P_ARP || eth_type < ETH_P_802_3_MIN || eth_type == 0xFEFE)
    {
        return false;
    }

    // Walk the headers until we find IP
    __u8 i = 0;
    while (i < 10 && !is_ip(eth_type))
    {
        switch (eth_type)
        {
        // Read inside VLAN headers
        case ETH_P_8021AD:
        case ETH_P_8021Q:
        {
            if SKB_OVERFLOW_OFFSET (dissector->start, dissector->end,
                                    offset, vlan_hdr)
            {
                return false;
            }
            struct vlan_hdr *vlan = (struct vlan_hdr *)(dissector->start + offset);
            dissector->current_vlan = vlan->h_vlan_TCI;
            eth_type = bpf_ntohs(vlan->h_vlan_encapsulated_proto);
            offset += sizeof(struct vlan_hdr);
            // VLAN Redirection is requested, so lookup a detination and
            // switch the VLAN tag if required
            if (vlan_redirect)
            {
#ifdef VERBOSE
                bpf_debug("Searching for redirect %u:%u",
                          dissector->ctx->ingress_ifindex,
                          bpf_ntohs(dissector->current_vlan));
#endif
                __u32 key = (dissector->ctx->ingress_ifindex << 16) |
                            bpf_ntohs(dissector->current_vlan);
                struct bifrost_vlan *vlan_info = NULL;
                vlan_info = bpf_map_lookup_elem(&bifrost_vlan_map, &key);
                if (vlan_info)
                {
#ifdef VERBOSE
                    bpf_debug("Redirect to VLAN %u",
                              bpf_htons(vlan_info->redirect_to));
#endif
                    vlan->h_vlan_TCI = bpf_htons(vlan_info->redirect_to);
                }
            }
        }
        break;

        // Handle PPPoE
        case ETH_P_PPP_SES:
        {
            if SKB_OVERFLOW_OFFSET (dissector->start, dissector->end,
                                    offset, pppoe_proto)
            {
                return false;
            }
            struct pppoe_proto *pppoe = (struct pppoe_proto *)(dissector->start + offset);
            __u16 proto = bpf_ntohs(pppoe->proto);
            switch (proto)
            {
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

        // WARNING/TODO: Here be dragons; this needs testing.
        case ETH_P_MPLS_UC:
        case ETH_P_MPLS_MC:
        {
            if SKB_OVERFLOW_OFFSET (dissector->start, dissector->end,
                                    offset, mpls_label)
            {
                return false;
            }
            struct mpls_label *mpls = (struct mpls_label *)(dissector->start + offset);
            // Are we at the bottom of the stack?
            offset += 4; // 32-bits
            if (mpls->entry & MPLS_LS_S_MASK)
            {
                // We've hit the bottom
                if SKB_OVERFLOW_OFFSET (dissector->start, dissector->end,
                                        offset, iphdr)
                {
                    return false;
                }
                struct iphdr *iph = (struct iphdr *)(dissector->start + offset);
                switch (iph->version)
                {
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

        // We found something we don't know how to handle - bail out
        default:
            return false;
        }
        ++i;
    }

    dissector->l3offset = offset;
    dissector->eth_type = eth_type;
    return true;
}

static __always_inline struct tcphdr *get_tcp_header(struct dissector_t *dissector)
{
    if (dissector->eth_type == ETH_P_IP && dissector->ip_header.iph->protocol == IPPROTO_TCP)
    {
        return (struct tcphdr *)((char *)dissector->ip_header.iph + (dissector->ip_header.iph->ihl * 4));
    }
    else if (dissector->eth_type == ETH_P_IPV6 && dissector->ip_header.ip6h->nexthdr == IPPROTO_TCP)
    {
        return (struct tcphdr *)(dissector->ip_header.ip6h + 1);
    }
    return NULL;
}

static __always_inline struct udphdr *get_udp_header(struct dissector_t *dissector)
{
    if (dissector->eth_type == ETH_P_IP)
    {
        return (struct udphdr *)((char *)dissector->ip_header.iph + (dissector->ip_header.iph->ihl * 4));
    }
    else if (dissector->eth_type == ETH_P_IPV6)
    {
        return (struct udphdr *)(dissector->ip_header.ip6h + 1);
    }
    return NULL;
}

static __always_inline struct icmphdr *get_icmp_header(struct dissector_t *dissector)
{
    if (dissector->eth_type == ETH_P_IP)
    {
        return (struct icmphdr *)((char *)dissector->ip_header.iph + (dissector->ip_header.iph->ihl * 4));
    }
    else if (dissector->eth_type == ETH_P_IPV6)
    {
        return (struct icmphdr *)(dissector->ip_header.ip6h + 1);
    }
    return NULL;
}

#define DIS_TCP_FIN 1
#define DIS_TCP_SYN 2
#define DIS_TCP_RST 4
#define DIS_TCP_PSH 8
#define DIS_TCP_ACK 16
#define DIS_TCP_URG 32
#define DIS_TCP_ECE 64
#define DIS_TCP_CWR 128

#define BITCHECK(flag) (dissector->tcp_flags & flag)

static __always_inline void snoop(struct dissector_t *dissector)
{
    switch (dissector->ip_protocol)
    {
    case IPPROTO_TCP:
    {
        struct tcphdr *hdr = get_tcp_header(dissector);
        if (hdr != NULL)
        {
            if (hdr + 1 > dissector->end)
            {
                return;
            }
            dissector->src_port = hdr->source;
            dissector->dst_port = hdr->dest;
            __u8 flags = 0;
            if (hdr->fin) flags |= DIS_TCP_FIN;
            if (hdr->syn) flags |= DIS_TCP_SYN;
            if (hdr->rst) flags |= DIS_TCP_RST;
            if (hdr->psh) flags |= DIS_TCP_PSH;
            if (hdr->ack) flags |= DIS_TCP_ACK;
            if (hdr->urg) flags |= DIS_TCP_URG;
            if (hdr->ece) flags |= DIS_TCP_ECE;
            if (hdr->cwr) flags |= DIS_TCP_CWR;

            dissector->tcp_flags = flags;
            dissector->window = hdr->window;
            dissector->sequence = hdr->seq;

            parse_tcp_ts(hdr, dissector->end, &dissector->tsval, &dissector->tsecr);
        }
    } break;
    case IPPROTO_UDP:
    {
        struct udphdr *hdr = get_udp_header(dissector);
        if (hdr != NULL)
        {
            if (hdr + 1 > dissector->end)
            {
                bpf_debug("UDP header past end");
                return;
            }
            dissector->src_port = hdr->source;
            dissector->dst_port = hdr->dest;
        }
    } break;
    case IPPROTO_ICMP:
    {
        struct icmphdr *hdr = get_icmp_header(dissector);
        if (hdr != NULL)
        {
            if ((char *)hdr + sizeof(struct icmphdr) > dissector->end)
            {
                bpf_debug("ICMP header past end");
                return;
            }
            dissector->ip_protocol = 1;
            dissector->src_port = bpf_ntohs(hdr->type);
            dissector->dst_port = bpf_ntohs(hdr->code);
        }    
    } break;
    }
}

// Searches for an IP header.
static __always_inline bool dissector_find_ip_header(
    struct dissector_t *dissector)
{
    switch (dissector->eth_type)
    {
    case ETH_P_IP:
    {
        if (dissector->start + dissector->l3offset + sizeof(struct iphdr) >
            dissector->end)
        {
            return false;
        }
        dissector->ip_header.iph = dissector->start + dissector->l3offset;
        if (dissector->ip_header.iph + 1 > dissector->end)
            return false;
        encode_ipv4(dissector->ip_header.iph->saddr, &dissector->src_ip);
        encode_ipv4(dissector->ip_header.iph->daddr, &dissector->dst_ip);
        dissector->ip_protocol = dissector->ip_header.iph->protocol;
        dissector->tos = dissector->ip_header.iph->tos;
        snoop(dissector);

        return true;
    }
    break;
    case ETH_P_IPV6:
    {
        if (dissector->start + dissector->l3offset +
                sizeof(struct ipv6hdr) >
            dissector->end)
        {
            return false;
        }
        dissector->ip_header.ip6h = dissector->start + dissector->l3offset;
        if (dissector->ip_header.iph + 1 > dissector->end)
            return false;
        encode_ipv6(&dissector->ip_header.ip6h->saddr, &dissector->src_ip);
        encode_ipv6(&dissector->ip_header.ip6h->daddr, &dissector->dst_ip);
        dissector->ip_protocol = dissector->ip_header.ip6h->nexthdr;
        dissector->tos = dissector->ip_header.ip6h->flow_lbl[0]; // Is this right?
        snoop(dissector);
        return true;
    }
    break;
    default:
        return false;
    }
}
