#pragma once

// Define VERBOSE or TRACING if you want to fill
// `/sys/kernel/debug/tracing/trace_pipe` with per-packet debug
// info. You usually don't want this in production.
//#define VERBOSE 1

#if defined(VERBOSE) || defined(TRACING)
#define bpf_debug(fmt, ...)                        \
	({                                             \
		char ____fmt[] = " " fmt;             \
		bpf_trace_printk(____fmt, sizeof(____fmt), \
						 ##__VA_ARGS__);           \
	})
#else
#define bpf_debug(fmt, ...) ((void)0)
#endif
