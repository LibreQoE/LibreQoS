import {subscribeWS} from "./pubsub/ws";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {PacketsPerSecondBar} from "./graphs/packets_bar";
import {ShapedUnshapedPie} from "./graphs/shaped_unshaped_pie";
import {ThroughputRingBufferGraph} from "./graphs/throughput_ring_graph";

let tpBits = null;
let tpPackets = null;
let tpShaped = null;
let tpRing = null;

function onMessage(msg) {
    switch (msg.event) {
        case "join": {
            if (msg.channel === "throughput") {
                tpBits = new BitsPerSecondGauge("tpBits");
                tpPackets = new PacketsPerSecondBar("tpPackets");
                tpShaped = new ShapedUnshapedPie("tpShaped");
                tpRing = new ThroughputRingBufferGraph("tpRing");
            }
        }
            break;
        case "throughput": {
            tpBits.update(msg.data.bps[0], msg.data.bps[1], msg.data.max[0], msg.data.max[1]);
            tpPackets.update(msg.data.pps[0], msg.data.pps[1]);
            tpShaped.update(msg.data.shaped_bps[0] + msg.data.shaped_bps[1], msg.data.bps[0] + msg.data.bps[1]);
            tpRing.update(msg.data.shaped_bps, msg.data.bps);
        }
            break;
    }
}

subscribeWS(["throughput"], onMessage);