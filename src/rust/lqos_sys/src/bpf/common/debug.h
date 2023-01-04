#pragma once

// Define VERBOSE if you want to fill
// `/sys/kernel/debug/tracing/trace_pipe` with per-packet debug
// info. You usually don't want this.
//#define VERBOSE 1

#define bpf_debug(fmt, ...)                        \
	({                                             \
		char ____fmt[] = " " fmt;             \
		bpf_trace_printk(____fmt, sizeof(____fmt), \
						 ##__VA_ARGS__);           \
	})