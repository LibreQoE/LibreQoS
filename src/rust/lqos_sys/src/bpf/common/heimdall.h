#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "maximums.h"
#include "debug.h"
#include "dissector.h"

// Array containing one element, the Heimdall configuration
struct heimdall_config_t
{
    __u32 monitor_mode; // 0 = Off, 1 = Targets only, 2 = Analysis Mode
};

struct
{
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __type(key, __u32);
    __type(value, struct heimdall_config_t);
    __uint(max_entries, 2);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} heimdall_config SEC(".maps");

struct
{
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, struct in6_addr);
    __type(value, __u32);
    __uint(max_entries, 64);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} heimdall_watching SEC(".maps");

struct heimdall_key
{
    struct in6_addr src;
    struct in6_addr dst;
    __u8 ip_protocol;
    __u16 src_port;
    __u16 dst_port;
};

struct heimdall_data
{
    __u64 last_seen;
    __u64 bytes;
    __u64 packets;
    __u8 tos;
    __u8 reserved[3];
};

struct
{
    __uint(type, BPF_MAP_TYPE_LRU_PERCPU_HASH);
    __type(key, struct heimdall_key);
    __type(value, struct heimdall_data);
    __uint(max_entries, MAX_FLOWS);
    __uint(pinning, LIBBPF_PIN_BY_NAME);
} heimdall SEC(".maps");

static __always_inline __u8 get_heimdall_mode()
{
    __u32 index = 0;
    struct heimdall_config_t *cfg = (struct heimdall_config_t *)bpf_map_lookup_elem(&heimdall_config, &index);
    if (cfg)
    {
        return cfg->monitor_mode;
    }
    else
    {
        return 0;
    }
}

static __always_inline bool is_heimdall_watching(struct dissector_t *dissector)
{
    __u32 *watching = bpf_map_lookup_elem(&heimdall_watching, &dissector->src_ip);
    if (watching)
        return true;
    watching = bpf_map_lookup_elem(&heimdall_watching, &dissector->dst_ip);
    if (watching)
        return true;
    return false;
}

static __always_inline void update_heimdall(struct dissector_t *dissector, __u32 size, int dir)
{
    // Don't report any non-ICMP without ports
    if (dissector->ip_protocol != 1 && (dissector->src_port == 0 || dissector->dst_port == 0))
        return;
    // Don't report ICMP with invalid numbers
    if (dissector->ip_protocol == 1 && dissector->src_port > 18) return;
    struct heimdall_key key = {0};
    key.src = dissector->src_ip;
    key.dst = dissector->dst_ip;
    key.ip_protocol = dissector->ip_protocol;
    key.src_port = bpf_ntohs(dissector->src_port);
    key.dst_port = bpf_ntohs(dissector->dst_port);
    struct heimdall_data *counter = (struct heimdall_data *)bpf_map_lookup_elem(&heimdall, &key);
    if (counter)
    {
        counter->last_seen = bpf_ktime_get_boot_ns();
        counter->packets += 1;
        counter->bytes += size;
        if (dissector->tos != 0)
        {
            counter->tos = dissector->tos;
        }
    }
    else
    {
        struct heimdall_data counter = {0};
        counter.last_seen = bpf_ktime_get_boot_ns();
        counter.bytes = size;
        counter.packets = 1;
        counter.tos = dissector->tos;
        counter.reserved[0] = 0;
        counter.reserved[1] = 0;
        counter.reserved[2] = 0;
        if (bpf_map_update_elem(&heimdall, &key, &counter, BPF_NOEXIST) != 0)
        {
            bpf_debug("Failed to insert tracking");
        }
    }
}