#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "maximums.h"
#include "debug.h"
#include "dissector.h"

#define PACKET_OCTET_SIZE 128

// Array containing one element, the Heimdall configuration
struct heimdall_config_t
{
    __u32 monitor_mode; // 0 = Off, 1 = Targets only, 2 = Analysis Mode
};

// Pinned map containing the Heimdall config
struct
{
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __type(key, __u32);
    __type(value, struct heimdall_config_t);
    __uint(max_entries, 2);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} heimdall_config SEC(".maps");

// Pinned map containing the IP addresses (in packed IPv6 format)
// currently being watched by the Heimdall system.
struct
{
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, struct in6_addr);
    __type(value, __u32);
    __uint(max_entries, 64);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} heimdall_watching SEC(".maps");

// Perf map for communicating with userspace
struct {
	__uint(type, BPF_MAP_TYPE_RINGBUF);
	__uint(max_entries, 256 * 1024 /* 256 KB */);
} heimdall_events SEC(".maps");

// Basic event type to send to userspace when "hyperfocused" on a
// data flow.
struct heimdall_event {
    __u64 timetamp;
    struct in6_addr src;
    struct in6_addr dst;
    __u16 src_port;
    __u16 dst_port;
    __u8 ip_protocol;
    __u8 tos;
    __u32 size;
    __u8 tcp_flags;
    __u16 tcp_window;
    __u32 tsval;
    __u32 tsecr;
    __u8 dump[PACKET_OCTET_SIZE];
};

static __always_inline __u8 get_heimdall_mode()
{
    __u32 index = 0;
    struct heimdall_config_t *cfg = (struct heimdall_config_t *)bpf_map_lookup_elem(&heimdall_config, &index);
    if (cfg)
    {
        #ifdef VERBOSE
        bpf_debug("Heimdall Mode: %d", cfg->monitor_mode);
        #endif
        return cfg->monitor_mode;
    }
    else
    {
        return 0;
    }
}

static __always_inline bool is_heimdall_watching(struct dissector_t *dissector, int effective_direction)
{
    if (effective_direction == 2) {
        __u32 * watching = (__u32 *)bpf_map_lookup_elem(&heimdall_watching, &dissector->src_ip);
        if (watching) {
            return true;
        }
    } else {
        __u32 * watching = (__u32 *)bpf_map_lookup_elem(&heimdall_watching, &dissector->dst_ip);
        if (watching) {
            return true;
        }
    }
    return false;
}

static __always_inline void update_heimdall(struct dissector_t *dissector, __u32 size, __u8 mode)
{
    if (mode == 2) {
        struct heimdall_event event = {0};
        event.timetamp = dissector->now;
        event.src = dissector->src_ip;
        event.dst = dissector->dst_ip;
        event.src_port = dissector->src_port;
        event.dst_port = dissector->dst_port;
        event.ip_protocol = dissector->ip_protocol;
        event.tos = dissector->tos;
        event.size = size;
        event.tcp_flags = dissector->tcp_flags;
        event.tcp_window = dissector->window;
        event.tsval = dissector->tsval;
        event.tsecr = dissector->tsecr;
        //if (size > PACKET_OCTET_SIZE) size = PACKET_OCTET_SIZE;
        bpf_probe_read_kernel(&event.dump, PACKET_OCTET_SIZE, dissector->start);
        bpf_ringbuf_output(&heimdall_events, &event, sizeof(event), 0);
    }
    
    // Commented out because we don't really care - some will be missed
    // during very heavy load.
    //if (err != 0) {
    //    bpf_debug("Failed to send perf event %d", err);
    //}
}