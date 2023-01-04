#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "../common/skb_safety.h"
#include "../common/debug.h"
#include "../common/ip_hash.h"
#include "dissector.h"

// Structure holding packet dissection information (obtained at the TC level)
struct tc_dissector_t
{
    // Pointer to the SKB context.
    struct __sk_buff *ctx;
    // Pointer to the data start
    void *start;
    // Pointer to the data end
    void *end;
    // Pointer to the Ethernet header once obtained (NULL until then)
    struct ethhdr *ethernet_header;
    // Ethernet packet type, once obtained
    __u16 eth_type;
    // Start of layer-3 data, once obtained
    __u32 l3offset;
    // IP header (either v4 or v6), once obtained.
    union iph_ptr ip_header;
    // Source IP, encoded by `ip_hash.h` functions.
    struct in6_addr src_ip;
    // Destination IP, encoded by `ip_hash.h` functions.
    struct in6_addr dst_ip;
    // Current VLAN detected.
    // TODO: This can probably be removed since the packet dissector
    // now finds this.
    __be16 current_vlan;
};

// Constructor for a dissector
// Connects XDP/TC SKB structure to a dissector structure.
// Arguments:
// * ctx - an xdp_md structure, passed from the entry-point
// * dissector - pointer to a local dissector object to be initialized
//
// Returns TRUE if all is good, FALSE if the process cannot be completed
static __always_inline bool tc_dissector_new(
    struct __sk_buff *ctx, 
    struct tc_dissector_t *dissector
) {
    dissector->ctx = ctx;
    dissector->start = (void *)(long)ctx->data;
    dissector->end = (void *)(long)ctx->data_end;
    dissector->ethernet_header = (struct ethhdr *)NULL;
    dissector->l3offset = 0;
    dissector->current_vlan = bpf_htons(ctx->vlan_tci);

    // Check that there's room for an ethernet header
    if SKB_OVERFLOW (dissector->start, dissector->end, ethhdr)
    {
        return false;
    }
    dissector->ethernet_header = (struct ethhdr *)dissector->start;

    return true;
}

// Search a context to find the layer-3 offset.
static __always_inline bool tc_dissector_find_l3_offset(
    struct tc_dissector_t *dissector
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
            //bpf_debug("TC Found VLAN");
            struct vlan_hdr *vlan = (struct vlan_hdr *)
                (dissector->start + offset);
            // Calculated from the SKB
            //dissector->current_vlan = vlan->h_vlan_TCI;
            eth_type = bpf_ntohs(vlan->h_vlan_encapsulated_proto);
            offset += sizeof(struct vlan_hdr);
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

// Locate the IP header if present
static __always_inline bool tc_dissector_find_ip_header(
    struct tc_dissector_t *dissector
) {
    switch (dissector->eth_type)
    {
    case ETH_P_IP:
    {
        if (dissector->start + dissector->l3offset + 
            sizeof(struct iphdr) > dissector->end) {
                return false;
        }
        dissector->ip_header.iph = dissector->start + dissector->l3offset;
        if (dissector->ip_header.iph + 1 > dissector->end) {
            return false;
        }
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