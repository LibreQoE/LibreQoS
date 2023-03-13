#include "wrapper.h"
#include "common/maximums.h"

struct lqos_kern * lqos_kern_open() {
    return lqos_kern__open();
}

int lqos_kern_load(struct lqos_kern * skel) {
    return lqos_kern__load(skel);
}

extern __u64 max_tracker_ips() {
	return MAX_TRACKED_IPS;
}

static int libbpf_print_fn(enum libbpf_print_level level, const char *format, va_list args)
{
 return 0;
}

void do_not_print() {
	libbpf_set_print(libbpf_print_fn);
}

/////////////////////////////////////////////////////////////////////////////////////
// The following is derived from
// https://github.com/xdp-project/bpf-examples/blob/master/tc-policy/tc_txq_policy.c
// It needs converting to Rust, but I wanted to get something
// working relatively quickly.

#include <linux/bpf.h>
#include <bpf/libbpf.h>
#include <bpf/bpf.h>

#define EGRESS_HANDLE		0x1;
#define EGRESS_PRIORITY 	0xC02F;

int teardown_hook(int ifindex, const char * ifname, bool verbose)
{
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook,
			    .attach_point = BPF_TC_EGRESS,
			    .ifindex = ifindex);
	int err;

	/* When destroying the hook, any and ALL attached TC-BPF (filter)
	 * programs are also detached.
	 */
	err = bpf_tc_hook_destroy(&hook);
	if (err && verbose)
		fprintf(stderr, "Couldn't remove clsact qdisc on %s\n", ifname);

	if (verbose)
		printf("Flushed all TC-BPF egress programs (via destroy hook)\n");

	return err;
}

int tc_detach_egress(int ifindex, bool verbose, bool flush_hook, const char * ifname)
{
	int err;
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook, .ifindex = ifindex,
			    .attach_point = BPF_TC_EGRESS);
	DECLARE_LIBBPF_OPTS(bpf_tc_opts, opts_info);

	opts_info.handle   = EGRESS_HANDLE;
	opts_info.priority = EGRESS_PRIORITY;

	/* Check what program we are removing */
	err = bpf_tc_query(&hook, &opts_info);
	if (err && verbose) {
		fprintf(stderr, "No egress program to detach "
			"for ifindex %d (err:%d)\n", ifindex, err);
		return err;
	}
	if (verbose)
		printf("Detaching TC-BPF prog id:%d\n", opts_info.prog_id);

	/* Attempt to detach program */
	opts_info.prog_fd = 0;
	opts_info.prog_id = 0;
	opts_info.flags = 0;
	err = bpf_tc_detach(&hook, &opts_info);
	if (err && verbose) {
		fprintf(stderr, "Cannot detach TC-BPF program id:%d "
			"for ifindex %d (err:%d)\n", opts_info.prog_id,
			ifindex, err);
	}

	if (flush_hook)
		return teardown_hook(ifindex, ifname, verbose);

	return err;
}

int tc_attach_egress(int ifindex, bool verbose, struct lqos_kern *obj)
{
	int err = 0;
	int fd;
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook, .attach_point = BPF_TC_EGRESS);
	DECLARE_LIBBPF_OPTS(bpf_tc_opts, attach_egress);

	/* Selecting BPF-prog here: */
	//fd = bpf_program__fd(obj->progs.queue_map_4);
	fd = bpf_program__fd(obj->progs.tc_iphash_to_cpu);
	if (fd < 0) {
		fprintf(stderr, "Couldn't find egress program\n");
		err = -ENOENT;
		goto out;
	}
	attach_egress.prog_fd = fd;

	hook.ifindex = ifindex;

	err = bpf_tc_hook_create(&hook);
	if (err && err != -EEXIST) {
		fprintf(stderr, "Couldn't create TC-BPF hook for "
			"ifindex %d (err:%d)\n", ifindex, err);
		goto out;
	}
	if (verbose && err == -EEXIST) {
		printf("Success: TC-BPF hook already existed "
		       "(Ignore: \"libbpf: Kernel error message\")\n");
	}

	hook.attach_point = BPF_TC_EGRESS;
	attach_egress.flags    = BPF_TC_F_REPLACE;
	attach_egress.handle   = EGRESS_HANDLE;
	attach_egress.priority = EGRESS_PRIORITY;
	err = bpf_tc_attach(&hook, &attach_egress);
	if (err) {
		fprintf(stderr, "Couldn't attach egress program to "
			"ifindex %d (err:%d)\n", hook.ifindex, err);
		goto out;
	}

	if (verbose) {
		printf("Attached TC-BPF program id:%d\n",
		       attach_egress.prog_id);
	}
out:
	return err;
}

int teardown_hook_ingress(int ifindex, const char * ifname, bool verbose)
{
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook,
			    .attach_point = BPF_TC_INGRESS,
			    .ifindex = ifindex);
	int err;

	/* When destroying the hook, any and ALL attached TC-BPF (filter)
	 * programs are also detached.
	 */
	err = bpf_tc_hook_destroy(&hook);
	if (err && verbose)
		fprintf(stderr, "Couldn't remove clsact qdisc on %s\n", ifname);

	if (verbose)
		printf("Flushed all TC-BPF egress programs (via destroy hook)\n");

	return err;
}

int tc_detach_ingress(int ifindex, bool verbose, bool flush_hook, const char * ifname)
{
	int err;
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook, .ifindex = ifindex,
			    .attach_point = BPF_TC_INGRESS);
	DECLARE_LIBBPF_OPTS(bpf_tc_opts, opts_info);

	opts_info.handle   = EGRESS_HANDLE;
	opts_info.priority = EGRESS_PRIORITY;

	/* Check what program we are removing */
	err = bpf_tc_query(&hook, &opts_info);
	if (err && verbose) {
		fprintf(stderr, "No ingress program to detach "
			"for ifindex %d (err:%d)\n", ifindex, err);
		return err;
	}
	if (verbose)
		printf("Detaching TC-BPF prog id:%d\n", opts_info.prog_id);

	/* Attempt to detach program */
	opts_info.prog_fd = 0;
	opts_info.prog_id = 0;
	opts_info.flags = 0;
	err = bpf_tc_detach(&hook, &opts_info);
	if (err && verbose) {
		fprintf(stderr, "Cannot detach TC-BPF program id:%d "
			"for ifindex %d (err:%d)\n", opts_info.prog_id,
			ifindex, err);
	}

	if (flush_hook)
		return teardown_hook(ifindex, ifname, verbose);

	return err;
}

int tc_attach_ingress(int ifindex, bool verbose, struct lqos_kern *obj)
{
	int err = 0;
	int fd;
	DECLARE_LIBBPF_OPTS(bpf_tc_hook, hook, .attach_point = BPF_TC_INGRESS);
	DECLARE_LIBBPF_OPTS(bpf_tc_opts, attach_egress);

	/* Selecting BPF-prog here: */
	//fd = bpf_program__fd(obj->progs.queue_map_4);
	fd = bpf_program__fd(obj->progs.bifrost);
	if (fd < 0) {
		fprintf(stderr, "Couldn't find ingress program\n");
		err = -ENOENT;
		goto out;
	}
	attach_egress.prog_fd = fd;

	hook.ifindex = ifindex;

	err = bpf_tc_hook_create(&hook);
	if (err && err != -EEXIST) {
		fprintf(stderr, "Couldn't create TC-BPF hook for "
			"ifindex %d (err:%d)\n", ifindex, err);
		goto out;
	}
	if (verbose && err == -EEXIST) {
		printf("Success: TC-BPF hook already existed "
		       "(Ignore: \"libbpf: Kernel error message\")\n");
	}

	hook.attach_point = BPF_TC_INGRESS;
	attach_egress.flags    = BPF_TC_F_REPLACE;
	attach_egress.handle   = EGRESS_HANDLE;
	attach_egress.priority = EGRESS_PRIORITY;
	err = bpf_tc_attach(&hook, &attach_egress);
	if (err) {
		fprintf(stderr, "Couldn't attach egress program to "
			"ifindex %d (err:%d)\n", hook.ifindex, err);
		goto out;
	}

	if (verbose) {
		printf("Attached TC-BPF program id:%d\n",
		       attach_egress.prog_id);
	}
out:
	return err;
}
