#pragma once

// Kprobe-only helpers.
// IMPORTANT: Do not use these from XDP/TC paths; keep XDP verifier balance intact.

// Minimal VLAN header representation for scanning 802.1Q/AD tags.
struct vlan_hdr_kprobe {
    __be16 h_vlan_TCI;
    __be16 h_vlan_encapsulated_proto;
};

static __always_inline __be16 kprobe_scan_current_vlan(struct sk_buff *skb)
{
    unsigned char *data = (unsigned char *)BPF_CORE_READ(skb, data);
    if (!data) {
        return 0;
    }

    struct ethhdr eth = {0};
    if (bpf_probe_read_kernel(&eth, sizeof(eth), data) < 0) {
        return 0;
    }

    __u16 eth_type = bpf_ntohs(eth.h_proto);
    __u32 offset = sizeof(struct ethhdr);
    __be16 current_vlan = 0;

    // Keep this very small and bounded.
#pragma unroll
    for (int i = 0; i < 2; i++) {
        if (eth_type != ETH_P_8021AD && eth_type != ETH_P_8021Q) {
            break;
        }
        struct vlan_hdr_kprobe vlan = {0};
        if (bpf_probe_read_kernel(&vlan, sizeof(vlan), data + offset) < 0) {
            break;
        }
        current_vlan = vlan.h_vlan_TCI;
        eth_type = bpf_ntohs(vlan.h_vlan_encapsulated_proto);
        offset += sizeof(struct vlan_hdr_kprobe);
    }

    return current_vlan;
}

static __always_inline __be16 kprobe_current_vlan(struct sk_buff *skb)
{
    // Prefer the skb metadata tag (matches TC egress behavior).
    __u16 vlan_tci = BPF_CORE_READ(skb, vlan_tci);
    if (vlan_tci) {
        return bpf_htons(vlan_tci);
    }
    // Fall back to scanning the L2 header for non-accelerated VLAN tags.
    return kprobe_scan_current_vlan(skb);
}

// Determine effective direction for *egress* (dev_hard_start_xmit) packets.
// Returns 1 for download (to LAN), 2 for upload (to Internet).
static __always_inline __u8 kprobe_determine_effective_direction(
    struct sk_buff *skb,
    int ifindex,
    int to_internet_ifindex,
    int to_isp_ifindex,
    __be16 internet_vlan
)
{
    // On-a-stick: both directions share one ifindex, so use VLAN to infer egress.
    if (to_isp_ifindex < 0) {
        __be16 vlan = kprobe_current_vlan(skb);
        // Out to Internet => UPLOAD (2). Out to LAN/core => DOWNLOAD (1).
        return (vlan == internet_vlan) ? 2 : 1;
    }

    // Two-interface: infer direction from the egress interface.
    // to_internet_ifindex carries uploads; to_isp_ifindex carries downloads.
    return (ifindex == to_internet_ifindex) ? 2 : 1;
}
