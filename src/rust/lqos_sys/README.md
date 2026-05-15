## lqos_sys

This crate wraps the XDP component in externally callable Rust. This is
used by other systems to manage the XDP/TC eBPF system.

The `src/bpf` directory contains the C for the eBPF program, as well as
some wrapper helpers to bring it into Rust-space.

### Packet parsing policy

The XDP and TC dissectors fail open for packets they cannot parse safely. IPv4
packets must have a valid version, complete header, valid total length, and no
fragmentation before the BPF path uses transport headers.

IPv6 packets with Hop-by-Hop Options, Routing, Fragment, Authentication Header,
or Destination Options as the first Next Header pass unshaped and do not enter
flow tracking. Direct IPv6 packets without those first extension headers remain
eligible for IP lookup, while unsupported direct transport protocols skip
flow-tracking state.

MPLS parsing remains conservative. Stacked MPLS fixtures are treated as
pass-unshaped until a bounded, verifier-friendly parser is added and tested.
