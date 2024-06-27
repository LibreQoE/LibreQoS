import {BaseCombinedDashlet} from "./base_combined_dashlet";
import {Top10Downloaders} from "./top10_downloaders";
import {Top10FlowsRate} from "./top10flows_rate";

export class CombinedTopDashlet extends BaseCombinedDashlet {
    constructor(slot) {
        let dashlets = [
            new Top10Downloaders((slot * 1000) + 1),
            new Top10FlowsRate((slot * 1000) + 2),
        ]
        super(slot, dashlets);
    }

    title() {
        return "Top-10 Downloaders";
    }
}