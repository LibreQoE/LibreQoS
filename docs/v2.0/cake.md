# CAKE

By default, LibreQoS uses Common Applications Kept Enhanced (CAKE) using the diffserv4 parameter.

# DSCP

[https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml](https://www.iana.org/assignments/dscp-registry/dscp-registry.xhtml)

## Traffic Classes and DSCP Tags for Diffserv4

 * Latency Sensitive  (CS7, CS6, EF, VA, CS5, CS4)
 * Streaming Media    (AF4x, AF3x, CS3, AF2x, TOS4, CS2, TOS1)
 * Best Effort        (CS0, AF1x, TOS2, and those not specified)
 * Background Traffic (CS1)

## List of known Diffserv codepoints:

 *  Least Effort (CS1)
 *  Best Effort (CS0)
 *  Max Reliability & LLT "Lo" (TOS1)
 *  Max Throughput (TOS2)
 *  Min Delay (TOS4)
 *  LLT "La" (TOS5)
 *  Assured Forwarding 1 (AF1x) - x3
 *  Assured Forwarding 2 (AF2x) - x3
 *  Assured Forwarding 3 (AF3x) - x3
 *  Assured Forwarding 4 (AF4x) - x3
 *  Precedence Class 2 (CS2)
 *  Precedence Class 3 (CS3)
 *  Precedence Class 4 (CS4)
 *  Precedence Class 5 (CS5)
 *  Precedence Class 6 (CS6)
 *  Precedence Class 7 (CS7)
 *  Voice Admit (VA)
 *  Expedited Forwarding (EF)

## List of traffic classes in RFC 4594:

(roughly descending order of contended priority)

(roughly ascending order of uncontended throughput)

 *  Network Control (CS6,CS7)      - routing traffic
 *  Telephony (EF,VA)         - aka. VoIP streams
 *  Signalling (CS5)               - VoIP setup
 *  Multimedia Conferencing (AF4x) - aka. video calls
 *  Realtime Interactive (CS4)     - eg. games
 *  Multimedia Streaming (AF3x)    - eg. YouTube, NetFlix, Twitch
 *  Broadcast Video (CS3)
 *  Low Latency Data (AF2x,TOS4)      - eg. database
 *  Ops, Admin, Management (CS2,TOS1) - eg. ssh
 *  Standard Service (CS0 & unrecognised codepoints)
 *  High Throughput Data (AF1x,TOS2)  - eg. web traffic
 *  Low Priority Data (CS1)           - eg. BitTorrent
