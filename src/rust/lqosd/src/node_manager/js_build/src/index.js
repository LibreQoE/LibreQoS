import {subscribeWS} from "./ws";
import {DashboardGauge} from "./graphs";
import {PacketsBar} from "./graphs";

let tpbits = null;
let tppackets = null;

function onMessage(msg) {
    switch (msg.event) {
        case "join": {
            if (msg.channel === "throughput") {
                tpbits = new DashboardGauge("tpBits");
                tppackets = new PacketsBar("tpPackets");
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