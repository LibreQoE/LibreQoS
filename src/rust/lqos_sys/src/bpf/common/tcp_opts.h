#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <linux/pkt_cls.h>
#include <linux/in.h>
#include <linux/in6.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/ipv6.h>
#include <linux/tcp.h>
#include <bpf/bpf_endian.h>
#include <stdbool.h>

#define MAX_TCP_OPTIONS 10

/*
 * Parses the TSval and TSecr values from the TCP options field. If sucessful
 * the TSval and TSecr values will be stored at tsval and tsecr (in network
 * byte order).
 * Returns 0 if sucessful and -1 on failure
 */
static __always_inline int parse_tcp_ts(
    struct tcphdr *tcph, 
    void *data_end, 
    __u32 *tsval,
    __u32 *tsecr
) {
    int len = tcph->doff << 2;
    void *opt_end = (void *)tcph + len;
    __u8 *pos = (__u8 *)(tcph + 1); // Current pos in TCP options
    __u8 i, opt;
    volatile __u8
        opt_size; // Seems to ensure it's always read of from stack as u8

    if (tcph + 1 > data_end || len <= sizeof(struct tcphdr))
        return -1;
#pragma unroll // temporary solution until we can identify why the non-unrolled loop gets stuck in an infinite loop
    for (i = 0; i < MAX_TCP_OPTIONS; i++)
    {
        if (pos + 1 > opt_end || pos + 1 > data_end)
            return -1;

        opt = *pos;
        if (opt == 0) // Reached end of TCP options
            return -1;

        if (opt == 1)
        { // TCP NOP option - advance one byte
            pos++;
            continue;
        }

        // Option > 1, should have option size
        if (pos + 2 > opt_end || pos + 2 > data_end)
            return -1;
        opt_size = *(pos + 1);
        if (opt_size < 2) // Stop parsing options if opt_size has an invalid value
            return -1;

        // Option-kind is TCP timestap (yey!)
        if (opt == 8 && opt_size == 10)
        {
            if (pos + 10 > opt_end || pos + 10 > data_end)
                return -1;
            *tsval = bpf_ntohl(*(__u32 *)(pos + 2));
            *tsecr = bpf_ntohl(*(__u32 *)(pos + 6));
            return 0;
        }

        // Some other TCP option - advance option-length bytes
        pos += opt_size;
    }
    return -1;
}