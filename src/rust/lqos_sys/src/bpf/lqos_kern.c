/* SPDX-License-Identifier: GPL-2.0 */
// Minimal XDP program that passes all packets.
// Used to verify XDP functionality.
#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <linux/in6.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include <linux/pkt_cls.h>
#include <linux/pkt_sched.h> /* TC_H_MAJ + TC_H_MIN */
#include "common/debug.h"
#include "common/dissector.h"
#include "common/dissector_tc.h"
#include "common/maximums.h"
#include "common/throughput.h"
#include "common/lpm.h"
#include "common/cpu_map.h"
//#include "common/tcp_rtt.h"
#include "common/bifrost.h"
#include "common/heimdall.h"
#include "common/flows.h"

//#define VERBOSE 1
//#define TRACING 1

/* Theory of operation:
1. (Packet arrives at interface)
2. XDP (ingress) starts
  * Check that "direction" is set and any VLAN mappings
  * Dissect the packet to find VLANs and L3 offset
      * If VLAN redirection is enabled, change VLAN tags
      * to swap ingress/egress VLANs.
  * Perform LPM lookup to determine CPU destination
  * Track traffic totals
  * Perform CPU redirection
3. TC (ingress) starts
  * If interface redirection is enabled, bypass the bridge
    and redirect to the outbound interface.
  * If VLAN redirection has happened, ONLY redirect if
    there is a VLAN tag to avoid STP loops.
4. TC (egress) starts on the outbound interface
  * LPM lookup to find TC handle
  * If TCP, track RTT via ringbuffer and sampling
  * Send TC redirect to track at the appropriate handle.
*/

// Constant passed in during loading to either
// 1 (facing the Internet)
// 2 (facing the LAN)
// 3 (use VLAN mode, we're running on a stick)
// If it stays at 255, we have a configuration error.
int direction = 255;

// Also configured during loading. For "on a stick" support,
// these are mapped to the respective VLAN facing directions.
__be16 internet_vlan = 0; // Note: turn these into big-endian
__be16 isp_vlan = 0;

// Helpers from https://elixir.bootlin.com/linux/v5.4.153/source/tools/testing/selftests/bpf/progs/test_xdp_meta.c#L37
#define __round_mask(x, y) ((__typeof__(x))((y) - 1))
#define round_up(x, y) ((((x) - 1) | __round_mask(x, y)) + 1)
#define ctx_ptr(ctx, mem) (void *)(unsigned long)ctx->mem

// Structure for passing metadata from XDP to TC
struct metadata_pass_t {
    __u32 tc_handle; // The encoded TC handle
};

// XDP Entry Point
SEC("xdp")
int xdp_prog(struct xdp_md *ctx)
{
#ifdef TRACING
    __u64 started = bpf_ktime_get_ns();
#endif
#ifdef VERBOSE
    bpf_debug("XDP-RDR");
#endif
    if (direction == 255) {
        bpf_debug("Error: interface direction unspecified, aborting.");
        return XDP_PASS;
    }

    // Do we need to perform a VLAN redirect?
    bool vlan_redirect = false;
    { // Note: scope for removing temporaries from the stack
        __u32 my_interface = ctx->ingress_ifindex;
        struct bifrost_interface * redirect_info = NULL;
        redirect_info = bpf_map_lookup_elem(
            &bifrost_interface_map, 
            &my_interface
        );
        if (redirect_info) {
            // If we have a redirect, mark it - the dissector will
            // apply it
            vlan_redirect = true;
#ifdef VERBOSE
            bpf_debug("(XDP) VLAN redirection requested for this interface");
#endif
        }
    }

    struct dissector_t dissector = {0};
#ifdef VERBOSE
    bpf_debug("(XDP) START XDP");
    bpf_debug("(XDP) Running mode %u", direction);
    bpf_debug("(XDP) Scan VLANs: %u %u", internet_vlan, isp_vlan);
#endif
    // If the dissector is unable to figure out what's going on, bail
    // out.
    if (!dissector_new(ctx, &dissector)) return XDP_PASS;

    // Note that this step rewrites the VLAN tag if redirection
    // is requested.
    if (!dissector_find_l3_offset(&dissector, vlan_redirect)) return XDP_PASS;
    if (!dissector_find_ip_header(&dissector)) return XDP_PASS;
    u_int8_t effective_direction = determine_effective_direction(
        direction, 
        internet_vlan, 
        &dissector
    );

#ifdef VERBOSE
    bpf_debug("(XDP) Effective direction: %d", effective_direction);
#endif

#ifdef VERBOSE
    bpf_debug("(XDP) Spotted VLAN: %u", dissector.current_vlan);
#endif

    // Determine the lookup key by direction
    struct ip_hash_key lookup_key;
    struct ip_hash_info * ip_info = setup_lookup_key_and_tc_cpu(
        effective_direction, 
        &lookup_key, 
        &dissector
    );

    // Find the desired TC handle and CPU target
    __u32 tc_handle = 0;
    __u32 cpu = 0;
    if (ip_info) {
        tc_handle = ip_info->tc_handle;
        cpu = ip_info->cpu;
    }

    // Per-Flow RTT Tracking
    track_flows(&dissector, effective_direction);

    // Update the traffic tracking buffers
    track_traffic(
        effective_direction, 
        &lookup_key.address, 
        ctx->data_end - ctx->data, // end - data = length
        tc_handle,
        dissector.now
    );

    // Send on its way
    if (tc_handle != 0) {
        // Send data to Heimdall
        __u8 heimdall_mode = get_heimdall_mode();
        if (heimdall_mode > 0 && is_heimdall_watching(&dissector, effective_direction)) {
#ifdef VERBOSE
            bpf_debug("(XDP) Storing Heimdall Data");
#endif            
            update_heimdall(&dissector, ctx->data_end - ctx->data, heimdall_mode);
        }

        // Handle CPU redirection if there is one specified
        __u32 *cpu_lookup;
        cpu_lookup = bpf_map_lookup_elem(&cpus_available, &cpu);
        if (!cpu_lookup) {
            bpf_debug("Error: CPU %u is not mapped", cpu);
            return XDP_PASS; // No CPU found
        }
        __u32 cpu_dest = *cpu_lookup;

        // Can we adjust the metadata? We'll try to do so, and if we can store the
        // needed info there. Not all drivers support this, so it has to remain
        // optional. This call invalidates the ctx->data pointer, so it has to be
        // done last.
        int ret = bpf_xdp_adjust_meta(ctx, -round_up(ETH_ALEN, sizeof(struct metadata_pass_t)));
        if (ret < 0) {
            #ifdef VERBOSE
            bpf_debug("Error: unable to adjust metadata, ret: %d", ret);
            #endif
        } else {
            #ifdef VERBOSE
            bpf_debug("Metadata adjusted, ret: %d", ret);
            #endif

            __u8 *data_meta = ctx_ptr(ctx, data_meta);
            __u8 *data_end  = ctx_ptr(ctx, data_end);
            __u8 *data      = ctx_ptr(ctx, data);

            if (data + ETH_ALEN > data_end || data_meta + round_up(ETH_ALEN, 4) > data) {
                bpf_debug("Bounds error on the metadata");
                return XDP_DROP;
            }
            struct metadata_pass_t meta = (struct metadata_pass_t) {
                .tc_handle = tc_handle,
            };
            __builtin_memcpy(data_meta, &meta, sizeof(struct metadata_pass_t));
        }

        // Redirect based on CPU
#ifdef VERBOSE
        bpf_debug("(XDP) Zooming to CPU: %u", cpu_dest);
        bpf_debug("(XDP) Mapped to handle: %u", tc_handle);
#endif
        long redirect_result = bpf_redirect_map(&cpu_map, cpu_dest, 0);
#ifdef VERBOSE
        bpf_debug("(XDP) Redirect result: %u", redirect_result);
#endif

#ifdef TRACING
{
    __u64 now = bpf_ktime_get_ns();
    bpf_debug("(XDP) Exit time: %u", now - started);
}
#endif

        return redirect_result;
    }
	return XDP_PASS;
}

// TC-Egress Entry Point
SEC("tc")
int tc_iphash_to_cpu(struct __sk_buff *skb)
{
#ifdef TRACING
    __u64 started = bpf_ktime_get_ns();
#endif
#ifdef VERBOSE
    bpf_debug("TC-MAP");
#endif
    if (direction == 255) {
        bpf_debug("(TC) Error: interface direction unspecified, aborting.");
        return TC_ACT_OK;
    }
#ifdef VERBOSE
    bpf_debug("(TC) SKB VLAN TCI: %u", skb->vlan_tci);    
#endif

    __u32 cpu = bpf_get_smp_processor_id();

    // Lookup the queue
    {
        struct txq_config *txq_cfg;
        txq_cfg = bpf_map_lookup_elem(&map_txq_config, &cpu);
        if (!txq_cfg) return TC_ACT_SHOT;
        if (txq_cfg->queue_mapping != 0) {
            skb->queue_mapping = txq_cfg->queue_mapping;
        } else {
            bpf_debug("(TC) Misconf: CPU:%u no conf (curr qm:%d)\n", 
                cpu, skb->queue_mapping);
        }
    } // Scope to remove tcq_cfg when done with it

    // Do we have metadata?
    if (skb->data != skb->data_meta) {
        #ifdef VERBOSE
        bpf_debug("(TC) Metadata is present");
        #endif
        int size = skb->data_meta - skb->data;
        if (size < sizeof(struct metadata_pass_t)) {
            bpf_debug("(TC) Metadata too small");
        } else {
            // Use it here
            __u8 *data_meta = ctx_ptr(skb, data_meta);
            __u8 *data_end  = ctx_ptr(skb, data_end);
            __u8 *data      = ctx_ptr(skb, data);

	        if (data + ETH_ALEN > data_end || data_meta + round_up(ETH_ALEN, 4) > data)
            {
                bpf_debug("(TC) Bounds error on the metadata");
		        return TC_ACT_SHOT;
            }

            struct metadata_pass_t *meta = (struct metadata_pass_t *)data_meta;
            #ifdef VERBOSE
            bpf_debug("(TC) Metadata: CPU: %u, TC: %u", meta->cpu, meta->tc_handle);
            #endif
            if (meta->tc_handle != 0) {
                // We can short-circuit the redirect and bypass the second
                // LPM lookup! Yay!
                skb->priority = meta->tc_handle;
                #ifdef TRACING
                {
                    __u64 now = bpf_ktime_get_ns();
                    bpf_debug("(TC) Exit time: %u", now - started);
                }
                #endif
                return TC_ACT_OK;
            }
        }
    } else {
        #ifdef VERBOSE
        bpf_debug("(TC) No metadata present");
        #endif
    }

    // Once again parse the packet
    // Note that we are returning OK on failure, which is a little odd.
    // The reasoning being that if its a packet we don't know how to handle,
    // we probably don't want to drop it - to ensure that IS-IS, ARP, STP
    // and other packet types are still handled by the default queues.
    struct tc_dissector_t dissector = {0};
    if (!tc_dissector_new(skb, &dissector)) return TC_ACT_OK;
    if (!tc_dissector_find_l3_offset(&dissector)) return TC_ACT_OK;
    if (!tc_dissector_find_ip_header(&dissector)) return TC_ACT_OK;

    // Determine the lookup key by direction
    struct ip_hash_key lookup_key;
    int effective_direction = 0;
    struct ip_hash_info * ip_info = tc_setup_lookup_key_and_tc_cpu(
        direction, 
        &lookup_key, 
        &dissector, 
        internet_vlan, 
        &effective_direction
    );
#ifdef VERBOSE
    bpf_debug("(TC) effective direction: %d", effective_direction);
#endif

    if (ip_info && ip_info->tc_handle != 0) {
        // We found a matching mapped TC flow
#ifdef VERBOSE
        bpf_debug("(TC) Mapped to TC handle %x", ip_info->tc_handle);
#endif
        skb->priority = ip_info->tc_handle;
        #ifdef TRACING
        {
            __u64 now = bpf_ktime_get_ns();
            bpf_debug("(TC) Exit time: %u", now - started);
        }
        #endif
        return TC_ACT_OK;
    } else {
        // We didn't find anything
#ifdef VERBOSE
        bpf_debug("(TC) didn't map anything");
#endif
        #ifdef TRACING
        {
            __u64 now = bpf_ktime_get_ns();
            bpf_debug("(TC) Exit time: %u", now - started);
        }
        #endif
        return TC_ACT_OK;
    }

    return TC_ACT_OK;
}

// Helper function to call the bpf_redirect function and note
// errors from the TC-egress context.
static __always_inline long do_tc_redirect(__u32 target) {
    //bpf_debug("Packet would have been redirected to ifindex %u", target);
    //return TC_ACT_UNSPEC; // Don't actually redirect, we're testing
    long ret = bpf_redirect(target, 0);
    if (ret != TC_ACT_REDIRECT) {
        bpf_debug("(TC-IN) TC Redirect call failed");
        return TC_ACT_UNSPEC;
    } else {
        return ret;
    }
}

// TC-Ingress entry-point. eBPF Bridge ("bifrost")
SEC("tc")
int bifrost(struct __sk_buff *skb)
{
#ifdef VERBOSE
    bpf_debug("TC-Ingress invoked on interface: %u . %u", 
        skb->ifindex, skb->vlan_tci);
#endif
    // Lookup to see if we have redirection setup
    struct bifrost_interface * redirect_info = NULL;
    __u32 my_interface = skb->ifindex;
    redirect_info = bpf_map_lookup_elem(&bifrost_interface_map, &my_interface);
    if (redirect_info) {
#ifdef VERBOSE
        bpf_debug("(TC-IN) Redirect info: to: %u, scan vlans: %d", 
            redirect_info->redirect_to, redirect_info->scan_vlans);
#endif

        if (redirect_info->scan_vlans) {
            // We are in VLAN redirect mode. If VLAN redirection is required,
            // it already happened in the XDP stage (rewriting the header).
            //
            // We need to ONLY redirect if we have tagged packets, otherwise
            // we create STP loops and Bad Things (TM) happen.
            if (skb->vlan_tci > 0) {
#ifdef VERBOSE
                bpf_debug("(TC-IN) Redirecting back to same interface, \
                    VLAN %u", skb->vlan_tci);
#endif                
                return do_tc_redirect(redirect_info->redirect_to);
            } else {
#ifdef VERBOSE
                bpf_debug("(TC-IN) Not redirecting: No VLAN tag, bare \
                    redirect unsupported in VLAN mode.");
#endif
                return TC_ACT_UNSPEC;
            }
        } else {
            // We're in regular redirect mode. So if we aren't trying to send
            // a packet out via the interface it arrived, we can redirect.
            if (skb->ifindex == redirect_info->redirect_to) {
#ifdef VERBOSE
                bpf_debug("(TC-IN) Not redirecting: src and dst are the \
                same.");
#endif
                return TC_ACT_UNSPEC;
            } else {
                return do_tc_redirect(redirect_info->redirect_to);
            }
        }
    } else {
#ifdef VERBOSE
        bpf_debug("(TC-IN) No matching redirect record for interface %u", 
        my_interface);
#endif
    }
    return TC_ACT_UNSPEC;
}

/*
 * Structs for map iteration programs
 * See https://github.com/xdp-project/bpf-examples
 */
struct bpf_iter_meta {
	struct seq_file *seq;
	__u64 session_id;
	__u64 seq_num;
} __attribute__((preserve_access_index));

struct bpf_iter__bpf_map_elem {
	struct bpf_iter_meta *meta;
	struct bpf_map *map;
	void *key;
	void *value;
};

volatile const int NUM_CPUS = 0;

SEC("iter/bpf_map_elem")
int throughput_reader(struct bpf_iter__bpf_map_elem *ctx)
{
    // The sequence file
    struct seq_file *seq = ctx->meta->seq;
    void *counter = ctx->value;
    struct in6_addr *ip = ctx->key;
    __u32 num_cpus = NUM_CPUS;

    // Bail on end
    if (counter == NULL || ip == NULL) {
        return 0;
    }

    if (ctx->meta->seq_num == 0) {
        bpf_seq_write(seq, &num_cpus, sizeof(__u32));
        bpf_seq_write(seq, &num_cpus, sizeof(__u32)); // Repeat for padding
    }

    bpf_seq_write(seq, ip, sizeof(struct in6_addr));
    for (__u32 i=0; i<NUM_CPUS; i++) {
        struct host_counter * content = counter+(i*sizeof(struct host_counter));
        bpf_seq_write(seq, content, sizeof(struct host_counter));
    }

    //BPF_SEQ_PRINTF(seq, "%d %d\n", counter->download_bytes, counter->upload_bytes);
    return 0;
}

SEC("iter/bpf_map_elem")
int flow_reader(struct bpf_iter__bpf_map_elem *ctx)
{
    // The sequence file
    struct seq_file *seq = ctx->meta->seq;
    struct flow_data_t *counter = ctx->value;
    struct flow_key_t *ip = ctx->key;

    // Bail on end
    if (counter == NULL || ip == NULL) {
        return 0;
    }

    //BPF_SEQ_PRINTF(seq, "%d %d\n", counter->next_entry, counter->rtt[0]);
    bpf_seq_write(seq, ip, sizeof(struct flow_key_t));
    bpf_seq_write(seq, counter, sizeof(struct flow_data_t));
    return 0;
}

char _license[] SEC("license") = "GPL";
