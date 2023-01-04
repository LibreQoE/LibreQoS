#pragma once

// Maximum number of client IPs we are tracking
#define MAX_TRACKED_IPS 128000

// Maximum number of TC class mappings to support
#define IP_HASH_ENTRIES_MAX	128000

// Maximum number of supported CPUs
#define MAX_CPUS 1024

// Maximum number of TCP flows to track at once
#define MAX_FLOWS IP_HASH_ENTRIES_MAX*2

// Maximum number of packet pairs to track per flow.
#define MAX_PACKETS MAX_FLOWS
