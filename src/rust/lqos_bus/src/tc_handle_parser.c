// Imported from https://github.com/thebracket/cpumap-pping/blob/master/src/xdp_iphash_to_cpu_cmdline.c
// Because it uses strtoul and is based on the TC source, including it directly
// seemed like the path of least resistance.

#include <stdlib.h>
#include <stdbool.h>
#include <string.h>
#include <linux/types.h>
#include <linux/pkt_sched.h> /* TC macros */

/* Handle classid parsing based on iproute source */
int get_tc_classid(__u32 *h, const char *str)
{
	__u32 major, minor;
	char *p;

	major = TC_H_ROOT;
	if (strcmp(str, "root") == 0)
		goto ok;
	major = TC_H_UNSPEC;
	if (strcmp(str, "none") == 0)
		goto ok;
	major = strtoul(str, &p, 16);
	if (p == str) {
		major = 0;
		if (*p != ':')
			return -1;
	}
	if (*p == ':') {
		if (major >= (1<<16))
			return -1;
		major <<= 16;
		str = p+1;
		minor = strtoul(str, &p, 16);
		if (*p != 0)
			return -1;
		if (minor >= (1<<16))
			return -1;
		major |= minor;
	} else if (*p != 0)
		return -1;

ok:
	*h = major;
	return 0;
}