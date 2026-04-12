# Circuit QoE and RTT Methodology

## Page Purpose

Use this page to understand how LibreQoS currently calculates circuit-level RTT and QoE in the WebUI.

This page describes the circuit-focused methodology used by circuit pages and other circuit experience views. Site, node, and global views may use different rollups.

## What These Metrics Mean

- `RTT` is the representative round-trip latency for the circuit.
- `QoE` is the representative quality score for the circuit, based on latency and loss.
- Both metrics are designed to reflect the subscriber's overall experience across active destinations instead of letting one raw flow define the whole circuit.

## Circuit RTT

LibreQoS currently calculates circuit RTT in four stages:

1. Recent active flows are grouped by destination ASN within the circuit.
2. Very small flows are ignored for RTT contribution until they have transferred at least `128 KB` in that direction.
3. Each ASN builds its own RTT view from the RTT-bearing traffic LibreQoS can actually observe.
4. LibreQoS combines those ASN RTT values into one circuit RTT using a weighted median.

This gives more weight to meaningful active destinations while resisting single-flow or single-destination outliers.

## Circuit QoE

Circuit QoE uses the same ASN grouping as circuit RTT.

For each active ASN, LibreQoS:

- builds an RTT view from recent RTT-bearing traffic
- estimates transport loss using TCP retransmits where available
- applies the selected QoE profile from `qoo_profiles.json`

LibreQoS then combines the per-ASN QoE values into one circuit QoE score.

## How ASN Weighting Works

LibreQoS does not treat every flow equally.

Instead, current builds:

- group flows by destination ASN
- give more influence to ASNs carrying more active traffic
- reduce influence when RTT-visible traffic is only a small part of that ASN's total traffic
- cap the influence of any one ASN so a single destination cannot fully dominate the circuit score when enough distinct ASNs are active

This is intended to better represent subscriber experience when a circuit is talking to many destinations at once.

## Why This Is Better Than Raw Flow Weighting

Raw flow weighting can be misleading:

- many tiny flows can create noise
- one large streaming flow can overstate a problem that the ISP cannot influence
- QUIC-heavy traffic often has weaker RTT visibility than TCP

The ASN-adjusted method reduces those problems by:

- ignoring very small flows for RTT contribution
- weighting by destination groups instead of raw flow count
- discounting weak RTT evidence
- limiting how much any single destination group can control the result

## Important Limits

This is still an approximation of user experience, not a perfect classifier.

Keep these limits in mind:

- RTT visibility is better for TCP than for encrypted QUIC-heavy traffic.
- A single ASN can still represent multiple applications with different behavior.
- On circuits with only a few active destinations, any cap on ASN influence has less room to work.
- Retransmit metrics shown elsewhere in the WebUI remain direct transport-health indicators and are not all ASN-adjusted.

## How To Interpret The Result

Use circuit RTT and QoE as experience signals, not as protocol-forensics.

Examples:

- If throughput is healthy, most destinations look good, and one streaming destination looks bad, QoE should degrade less than a raw flow-based method.
- If several major active destinations all look bad, the circuit RTT and QoE should still show that clearly.
- If RTT coverage is weak because most traffic is QUIC or otherwise opaque, treat the score as directional rather than absolute.

## Related Pages

- [Configure LibreQoS](configuration.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui.md)
- [TreeGuard](treeguard.md)
