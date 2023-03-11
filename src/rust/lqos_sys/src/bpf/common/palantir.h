#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "maximums.h"
#include "debug.h"
#include "dissector.h"

struct palantir_key {
    struct in6_addr src;
    struct in6_addr dst;
    __u8 ip_protocol;
    __u16 src_port;
    __u16 dst_port;
};

struct palantir_data {
    __u64 last_seen;
    __u64 bytes;
    __u64 packets;
    __u8 tos;
    __u8 reserved[3];
};

struct
{
	__uint(type, BPF_MAP_TYPE_LRU_PERCPU_HASH);
	__type(key, struct palantir_key);
	__type(value, struct palantir_data);
    __uint(max_entries, MAX_FLOWS);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} palantir SEC(".maps");

static __always_inline void update_palantir(struct dissector_t * dissector, __u32 size, int dir) {
    if (dissector->src_port == 0 || dissector->dst_port == 0) return;
    struct palantir_key key = {0};
    key.src = dissector->src_ip;
    key.dst = dissector->dst_ip;
    key.ip_protocol = dissector->ip_protocol;
    key.src_port = bpf_ntohs(dissector->src_port);
    key.dst_port = bpf_ntohs(dissector->dst_port);
    struct palantir_data * counter = (struct palantir_data *)bpf_map_lookup_elem(&palantir, &key);
    if (counter) {
        counter->last_seen = bpf_ktime_get_boot_ns();
        counter->packets += 1;
        counter->bytes += size;
        if (dissector->tos != 0) {
            counter->tos = dissector->tos;
        }
    } else {
        struct palantir_data counter = {0};
        counter.last_seen = bpf_ktime_get_boot_ns();
        counter.bytes = size;
        counter.packets = 1;
        counter.tos = dissector->tos;
        counter.reserved[0] = 0;
        counter.reserved[1] = 0;
        counter.reserved[2] = 0;
        if (bpf_map_update_elem(&palantir, &key, &counter, BPF_NOEXIST) != 0) {
            bpf_debug("Failed to insert tracking");
        }
    }
}