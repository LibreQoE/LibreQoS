import {subscribeWS} from "./pubsub/ws";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {PacketsPerSecondBar} from "./graphs/packets_bar";
import {ShapedUnshapedPie} from "./graphs/shaped_unshaped_pie";
import {ThroughputRingBufferGraph} from "./graphs/throughput_ring_graph";
import {RttHistogram} from "./graphs/rtt_histo";
import {FlowCountGraph} from "./graphs/flows_graph";

let tpBits = null;
let tpPackets = null;
let tpShaped = null;
let tpRing = null;
let rttHisto = null;
let tpFlows = null;

function onMessage(msg) {
    switch (msg.event) {
        case "join": {
            if (msg.channel === "throughput") {
                tpBits = new BitsPerSecondGauge("tpBits");
                tpPackets = new PacketsPerSecondBar("tpPackets");
                tpShaped = new ShapedUnshapedPie("tpShaped");
                tpRing = new ThroughputRingBufferGraph("tpRing");
            } else if (msg.channel === "rtt") {
                rttHisto = new RttHistogram("rttHisto");
            } else if (msg.channel === "flows") {
                tpFlows = new FlowCountGraph("tpFlows");
            }
        }
            break;
        case "throughput": {
            tpBits.update(msg.data.bps[0], msg.data.bps[1], msg.data.max[0], msg.data.max[1]);
            tpPackets.update(msg.data.pps[0], msg.data.pps[1]);
            let shaped = msg.data.shaped_bps[0] + msg.data.shaped_bps[1];
            let unshaped = msg.data.bps[0] + msg.data.bps[1];
            tpShaped.update(shaped, shaped - unshaped);
            tpRing.update(msg.data.shaped_bps, msg.data.bps);
        }
            break;
        case "histogram": {
            rttHisto.update(msg.data);
        } break;
        case "flows": {
            tpFlows.update(msg.data);
        }
            break;
    }
}

subscribeWS(["throughput", "rtt", "flows"], onMessage);