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
    // Total length (end - start)
    __u32 skb_len;
    // Ethernet header once found (NULL until then)
    struct ethhdr *ethernet_header;
    // Ethernet packet type once found (0 until then)
    __u16 eth_type;
    // Layer-3 offset if found (0 until then)
    __u32 l3offset;
    // IPv4/6 header once found
    union iph_ptr ip_header;
    // Source IP address, encoded by `ip_hash.h`
    struct in6_addr src_ip;
    // Destination IP address, encoded by `ip_hash.h`
    struct in6_addr dst_ip;
    // Current VLAN tag. If there are multiple tags, it will be
    // the INNER tag.
    __be16 current_vlan;
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

// Constructor for a dissector
// Connects XDP/TC SKB structure to a dissector structure.
// Arguments:
// * ctx - an xdp_md structure, passed from the entry-point
// * dissector - pointer to a local dissector object to be initialized
//
// Returns TRUE if all is good, FALSE if the process cannot be completed
static __always_inline bool dissector_new(
    struct xdp_md *ctx, 
    struct dissector_t *dissector
) {
    dissector->ctx = ctx;
    dissector->start = (void *)(long)ctx->data;
    dissector->end = (void *)(long)ctx->data_end;
    dissector->ethernet_header = (struct ethhdr *)NULL;
    dissector->l3offset = 0;
    dissector->skb_len = dissector->end - dissector->start;
    dissector->current_vlan = 0;

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
    bool vlan_redirect
) {
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

    // Fast return for ARP or non-802.3 ether types
    if (eth_type == ETH_P_ARP || eth_type < ETH_P_802_3_MIN)
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
            struct vlan_hdr *vlan = (struct vlan_hdr *)
                (dissector->start + offset);
            dissector->current_vlan = vlan->h_vlan_TCI;
            eth_type = bpf_ntohs(vlan->h_vlan_encapsulated_proto);
            offset += sizeof(struct vlan_hdr);
            // VLAN Redirection is requested, so lookup a detination and
            // switch the VLAN tag if required
            if (vlan_redirect) {
                #ifdef VERBOSE
                bpf_debug("Searching for redirect %u:%u", 
                    dissector->ctx->ingress_ifindex, 
                    bpf_ntohs(dissector->current_vlan)
                );
                #endif
                __u32 key = (dissector->ctx->ingress_ifindex << 16) | 
                    bpf_ntohs(dissector->current_vlan);
                struct bifrost_vlan * vlan_info = NULL;
                vlan_info = bpf_map_lookup_elem(&bifrost_vlan_map, &key);
                if (vlan_info) {
                    #ifdef VERBOSE
                    bpf_debug("Redirect to VLAN %u", 
                        bpf_htons(vlan_info->redirect_to)
                    );
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
            struct pppoe_proto *pppoe = (struct pppoe_proto *)
                (dissector->start + offset);
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

// Searches for an IP header.
static __always_inline bool dissector_find_ip_header(
    struct dissector_t *dissector
) {
    switch (dissector->eth_type)
    {
    case ETH_P_IP:
    {
        if (dissector->start + dissector->l3offset + sizeof(struct iphdr) > 
            dissector->end) {
                return false;
        }
        dissector->ip_header.iph = dissector->start + dissector->l3offset;
        if (dissector->ip_header.iph + 1 > dissector->end)
            return false;
        encode_ipv4(dissector->ip_header.iph->saddr, &dissector->src_ip);
        encode_ipv4(dissector->ip_header.iph->daddr, &dissector->dst_ip);
        return true;
    }
    break;
    case ETH_P_IPV6:
    {
        if (dissector->start + dissector->l3offset + 
            sizeof(struct ipv6hdr) > dissector->end) {
                return false;
        }
        dissector->ip_header.ip6h = dissector->start + dissector->l3offset;
        if (dissector->ip_header.iph + 1 > dissector->end)
            return false;
        encode_ipv6(&dissector->ip_header.ip6h->saddr, &dissector->src_ip);
        encode_ipv6(&dissector->ip_header.ip6h->daddr, &dissector->dst_ip);
        return true;
    }
    break;
    default:
        return false;
    }
}