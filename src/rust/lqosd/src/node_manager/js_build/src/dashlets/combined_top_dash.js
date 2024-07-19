import {BaseCombinedDashlet} from "./base_combined_dashlet";
import {Top10Downloaders} from "./top10_downloaders";
import {Top10FlowsRate} from "./top10flows_rate";
import {Top10FlowsBytes} from "./top10flows_bytes";
import {Top10EndpointsByCountry} from "./endpoints_by_country";
import {IpProtocols} from "./ip_protocols";
import {EtherProtocols} from "./ether_protocols";

export class CombinedTopDashlet extends BaseCombinedDashlet {
    constructor(slot) {
        let dashlets = [
            new Top10Downloaders((slot * 1000) + 1),
            new Top10FlowsRate((slot * 1000) + 2),
            new Top10FlowsBytes((slot * 1000) + 3),
            new Top10EndpointsByCountry((slot * 1000) + 4),
            new IpProtocols((slot * 1000) + 5),
            new EtherProtocols((slot * 1000) + 6),
        ]
        super(slot, dashlets);
    }

    title() {
        return "Top-10 Downloaders";
    }

    canBeSlowedDown() {
        return true;
    }
}