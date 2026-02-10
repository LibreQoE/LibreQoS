import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {isColorBlindMode} from "../helpers/colorblind";
import {toNumber} from "../lq_js_common/helpers/scaling";

/**
 * Viridis color scale interpolation (0-1 input).
 * Returns hex color string.
 */
function lerpViridis(t) {
    // Viridis colormap sampled at 6 points, interpolated linearly
    const stops = [
        [68, 1, 84],    // #440154
        [59, 82, 139],  // #3B528B
        [33, 145, 140], // #21918C
        [94, 201, 98],  // #5EC962
        [253, 231, 37]  // #FDE725
    ];
    if (t <= 0) return "#440154";
    if (t >= 1) return "#FDE725";
    let idx = t * (stops.length - 1);
    let i = Math.floor(idx);
    let frac = idx - i;
    let c0 = stops[i], c1 = stops[i + 1];
    let r = Math.round(c0[0] + frac * (c1[0] - c0[0]));
    let g = Math.round(c0[1] + frac * (c1[1] - c0[1]));
    let b = Math.round(c0[2] + frac * (c1[2] - c0[2]));
    return "#" + ((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1);
}
import {isRedacted} from "../helpers/redact";

let lastRtt = {};

class TopTreeSankeyGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    nodeAlign: 'left',
                    type: 'sankey',
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        /*this.chart.on('click', (params) => {
            //console.log(params.name);
            let name = params.name;
            // If it contains a >, it's a link
            if (name.indexOf(" > ") === -1) {
                rootId = idOfNode(name);
            } else {
                rootId = idOfNode(params.data.source);
            }
        });*/
        //$("#btnRoot").click(() => { rootId = 0; });
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

export class TopTreeSankey extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this._lastTreeSummary = null;
        this._lastL2 = null;
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top Level Sankey";
    }

    tooltip() {
        return "<h5>Network Tree</h5><p>Summary of the top-level network tree, rendered as a Sankey. Ribbon width shows relative bandwidth, ribbon color percentage of capacity, and node color overall RTT.</p>";
    }

    subscribeTo() {
        return [ "TreeSummary", "TreeSummaryL2" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new TopTreeSankeyGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "TreeSummary") {
            this._lastTreeSummary = msg.data;
        } else if (msg.event === "TreeSummaryL2") {
            this._lastL2 = msg.data;
        } else {
            return;
        }

        if (!this._lastTreeSummary) return;

        let redact = isRedacted();

        // Build 2-level sankey if L2 is present, else fallback to 1-level
        const nodes = [];
        const links = [];
        nodes.push({ name: "Root", label: "Root" });

        const firstLevel = (this._lastTreeSummary || []).slice(1);

        // Map of parent id -> parent transport for quick lookup
        const parentMap = new Map();
        firstLevel.forEach((r) => parentMap.set(r[0], r[1]));

        // Helper to build node style from RTT
        const nodeStyleFromRtt = (name, rttsArr) => {
            let rtt = 0;
            if (rttsArr && rttsArr.length > 0) {
                rtt = toNumber(rttsArr[0], 0);
            } else if (lastRtt[name] !== undefined) {
                rtt = lastRtt[name];
            }
            lastRtt[name] = rtt;
            const rttPercent = Math.min(100, (rtt / 200) * 100);
            const color = isColorBlindMode()
                ? lerpViridis(rttPercent / 100)
                : lerpGreenToRedViaOrange(200 - rtt, 200);
            return { itemStyle: { color } };
        };

        if (this._lastL2 && Array.isArray(this._lastL2) && this._lastL2.length > 0) {
            // L2 data shape: [ [parent_id, [ [child_id, child_transport], ... ] ], ... ]
            for (const [parentId, children] of this._lastL2) {
                const p = parentMap.get(parentId);
                if (!p) continue;

                // Parent node styling
                const pName = p.name;
                const label = { fontSize: 9, color: "#999" };
                if (redact) label.fontFamily = "Illegible";

                const pStyle = nodeStyleFromRtt(pName, p.rtts);
                nodes.push({ name: pName, label, ...pStyle });

                // Compute Root->Parent value as sum of included child totals (down + up)
                let parentSum = 0;
                // Compute link color from parent's capacity percent (as before)
                const bytesAsMegabits = toNumber(p.current_throughput[0], 0) / 1000000;
                const maxBytes = toNumber(p.max_throughput[0], 0) / 8;
                const percent = Math.min(100, maxBytes > 0 ? (bytesAsMegabits / maxBytes) * 100 : 0);
                const capacityColor = isColorBlindMode()
                    ? lerpViridis(percent / 100)
                    : lerpGreenToRedViaOrange(100 - percent, 100);

                for (const [, child] of children) {
                    const cName = child.name;
                    const cTotal =
                        toNumber(child.current_throughput?.[0], 0) +
                        toNumber(child.current_throughput?.[1], 0);
                    if (cTotal <= 0) continue;
                    parentSum += cTotal;

                    // Child node and link parent->child
                    const cLabel = { fontSize: 9, color: "#999" };
                    if (redact) cLabel.fontFamily = "Illegible";
                    const cStyle = nodeStyleFromRtt(cName, child.rtts);
                    nodes.push({ name: cName, label: cLabel, ...cStyle });

                    // Link color for child can use child's capacity percent
                    const cBytesAsMegabits = toNumber(child.current_throughput?.[0], 0) / 1000000;
                    const cMaxBytes = toNumber(child.max_throughput?.[0], 0) / 8;
                    const cPercent = Math.min(100, cMaxBytes > 0 ? (cBytesAsMegabits / cMaxBytes) * 100 : 0);
                    const cCapacityColor = isColorBlindMode()
                        ? lerpViridis(cPercent / 100)
                        : lerpGreenToRedViaOrange(100 - cPercent, 100);

                    links.push({
                        source: pName,
                        target: cName,
                        value: cTotal,
                        lineStyle: { color: cCapacityColor },
                    });
                }

                if (parentSum > 0) {
                    links.push({
                        source: "Root",
                        target: pName,
                        value: parentSum,
                        lineStyle: { color: capacityColor },
                    });
                }
            }
            this.graph.update(nodes, links);
            return;
        }

        // Fallback to 1-level sankey using current logic
        firstLevel.forEach((r) => {
            let label = { fontSize: 9, color: "#999" };
            if (redact) label.fontFamily = "Illegible";

            let name = r[1].name;
            let bytes = toNumber(r[1].current_throughput[0], 0);
            let bytesAsMegabits = bytes / 1000000;
            let maxBytes = toNumber(r[1].max_throughput[0], 0) / 8;
            let percent = Math.min(100, maxBytes > 0 ? (bytesAsMegabits / maxBytes) * 100 : 0);
            let capacityColor = isColorBlindMode()
                ? lerpViridis(percent / 100)
                : lerpGreenToRedViaOrange(100 - percent, 100);

            if (r[1].rtts.length > 0) {
                lastRtt[name] = toNumber(r[1].rtts[0], 0);
            } else {
                lastRtt[name] = 0;
            }
            let rttPercent = Math.min(100, (lastRtt[name] / 200) * 100);
            let color = isColorBlindMode()
                ? lerpViridis(rttPercent / 100)
                : lerpGreenToRedViaOrange(200 - lastRtt[name], 200);

            if (bytesAsMegabits > 0) {
                nodes.push({ name: r[1].name, label, itemStyle: { color } });
                links.push({
                    source: "Root",
                    target: r[1].name,
                    value: toNumber(r[1].current_throughput[0], 0) + toNumber(r[1].current_throughput[1], 0),
                    lineStyle: { color: capacityColor },
                });
            }
        });
        this.graph.update(nodes, links);
    }
}
