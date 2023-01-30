#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>

// Merely removing a file does not mean that it is no longer
// in use. It has simply vanished from the filesystem
// Also trapping on errors and reporting them is helpful

static const char *pinned_maps[] = {
"/sys/fs/bpf/map_traffic",
"/sys/fs/bpf/map_ip_to_cpu_and_tc",
"/sys/fs/bpf/cpu_map",
"/sys/fs/bpf/cpus_available",
"/sys/fs/bpf/packet_ts",
"/sys/fs/bpf/flow_state",
"/sys/fs/bpf/rtt_tracker",
"/sys/fs/bpf/map_ip_to_cpu_and_tc_recip",
"/sys/fs/bpf/tc/globals/map_txq_config",
"/sys/fs/bpf/bifrost_interface_map",
"/sys/fs/bpf/bifrost_vlan_map",
NULL
};

int remove_pinned() {
	const char **p = pinned_maps;
	int err=0;
	for (; *p != 0; p++) {
		printf("Deleting file %s\n", *p);
		if(unlink(*p) != 0 ) {
			switch(errno) {
			case EACCES:
			case EBUSY:
			case EFAULT:
			case EIO:
			case EISDIR:
			case ELOOP:
			case ENAMETOOLONG:
			case ENOMEM:
			case ENOTDIR:
			case EPERM:
			// Specific to unlinkat
			case EBADF:
			case EINVAL: // invaldd flags
			default: perror("bad operation"); err++;
			}
		}
	}
	return err;
}

int main(int argc, char **argv) {
		exit(remove_pinned());
}
