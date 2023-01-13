#pragma once

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>
#include <linux/if_ether.h>
#include <stdbool.h>
#include "maximums.h"
#include "debug.h"

// Defines a bridge-free redirect interface.
struct bifrost_interface {
	// The interface index to which this interface (from the key)
	// should redirect.
    __u32 redirect_to;
	// Should VLANs be scanned (for VLAN redirection)?
	// > 0 = true. 32-bit for padding reasons.
    __u32 scan_vlans;
};

// Hash map defining up to 64 interface redirects.
// Keyed on the source interface index, value is a bifrost_interface
// structure.
struct {
	__uint(type, BPF_MAP_TYPE_HASH);
	__uint(max_entries, 64);
	__type(key, __u32);
	__type(value, struct bifrost_interface);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} bifrost_interface_map SEC(".maps");

// TODO: This could be a u32 if we don't need any additional info.
// Which VLAN should the keyed VLAN be redirected to?
struct bifrost_vlan {
    __u32 redirect_to;
};

// Hash map of VLANs that should be redirected.
struct {
	__uint(type, BPF_MAP_TYPE_HASH);
	__uint(max_entries, 64);
	__type(key, __u32);
	__type(value, struct bifrost_vlan);
	__uint(pinning, LIBBPF_PIN_BY_NAME);
} bifrost_vlan_map SEC(".maps");