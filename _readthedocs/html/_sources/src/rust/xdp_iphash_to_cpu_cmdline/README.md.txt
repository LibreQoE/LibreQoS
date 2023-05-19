Note that this is *almost* compatible with the previous version. The syntax has changed so that the command must go first, and doesn't require a `--`.

```
Usage: xdp_iphash_to_cpu_cmdline [COMMAND]

Commands:
  add    Add an IP Address (v4 or v6) to the XDP/TC mapping system
  del    Remove an IP address (v4 or v6) from the XDP/TC mapping system
  clear  Clear all mapped IPs
  list   List all mapped IPs
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help information
```

Examples:

```
xdp_iphash_to_cpu_cmdline list
xdp_iphash_to_cpu_cmdline add --ip 192.168.100.1 --classid 5:3 --cpu 4
xdp_iphash_to_cpu_cmdline del 192.168.100.1
xdp_iphash_to_cpu_cmdline clear
```
