import {subscribeWS} from "./pubsub/ws";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {PacketsPerSecondBar} from "./graphs/packets_bar";

let tpbits = null;
let tppackets = null;

function onMessage(msg) {
    switch (msg.event) {
        case "join": {
            if (msg.channel === "throughput") {
                tpbits = new BitsPerSecondGauge("tpBits");
                tppackets = new PacketsPerSecondBar("tpPackets");
            }
        }
            break;
        case "throughput": {
            tpbits.update(msg.data.bps[0], msg.data.bps[1], msg.data.max[0], msg.data.max[1]);
            tppackets.update(msg.data.pps[0], msg.data.pps[1]);
        }
            break;
    }
}

subscribeWS(["throughput"], onMessage);