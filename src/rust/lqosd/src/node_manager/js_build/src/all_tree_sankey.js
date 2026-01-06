import { DashboardGraph } from "./graphs/dashboard_graph";
import {lerpGreenToRedViaOrange} from "./helpers/scaling";
import {isColorBlindMode} from "./helpers/colorblind";
import {toNumber} from "./lq_js_common/helpers/scaling";
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
import {isRedacted} from "./helpers/redact";
import {GenericRingBuffer} from "./helpers/ringbuffer";
import {trimStringWithElipsis} from "./helpers/strings_help";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

class AllTreeSankeyGraph extends GenericRingBuffer {
    constructor() {
        super(10);
    }

    onTick(graph) {
        let self = this;
        listenOnce("NetworkTree", (msg) => {
            const data = msg && msg.data ? msg.data : [];
            // Maintain a 10-second ringbuffer of recent data
            this.push(data);

            let redact = isRedacted();
            let nodes = [];
            let links = [];

            // Build the basic tree from the current head, to ensure
            // that we're displaying the most recent nodes.
            let head = self.getHead();
            if (head === undefined) { return }
            let startDepth = head[rootId][1].parents.length - 1;
            for (let i=0; i<head.length; i++) {
                let depth = head[i][1].parents.length - startDepth;
                if (depth > maxDepth) {
                    continue;
                }
                // If head[i][1].parents does not contain rootId, skip
                if (rootId !== 0 && !head[i][1].parents.includes(rootId)) {
                    continue;
                }
                let name = head[i][1].name;
                let bytes = toNumber(head[i][1].current_throughput[0], 0);
                let bytesAsMegabits = bytes / 1000000;
                let maxBytes = toNumber(head[i][1].max_throughput[0], 0) / 8;
                let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
                // Use appropriate color scale based on color blind mode
                let capacityColor = isColorBlindMode() 
                    ? lerpViridis(percent / 100)
                    : lerpGreenToRedViaOrange(100 - percent, 100);

                // Use appropriate color scale for node
                let color = capacityColor;

                let label = {
                    fontSize: 10,
                    color: "#999",
                    formatter: (params) => {
                        // Trim to 10 chars with elipsis
                        return trimStringWithElipsis(params.name.replace("(Generated Site) ", ""), 14);
                    }
                };
                if (redact) {
                    label.fontFamily = "Illegible";
                }

                nodes.push({
                    name: head[i][1].name,
                    label: label,
                    itemStyle: {
                        color: color
                    },
                    n: 1,
                });

                if (i > 0) {
                    let immediateParent = head[i][1].immediate_parent;
                    links.push({
                        source: head[immediateParent][1].name,
                        target: head[i][1].name,
                        value: Math.min(1, toNumber(head[i][1].current_throughput[0], 0)),
                        lineStyle: {
                            color: capacityColor,
                        },
                        maxBytes: maxBytes,
                        n: 1,
                    });
                }
            }

            // Now we iterate over the entire ringbuffer to accumulate data over the period
            // of the ringbuffer.
            self.iterate((data) => {
                for (let i=0; i<data.length; i++) {
                    // Search for links that match so we can update the value
                    if (i > 0) {
                        let immediateParent = data[i][1].immediate_parent;
                        let link = links.find((link) => { return link.source === data[immediateParent][1].name && link.target === data[i][1].name; });
                        if (link !== undefined) {
                            link.value += toNumber(data[i][1].current_throughput[0], 0);
                            link.n++;
                        }
                    }
                }
            });

            // Now go through the links and average the values, recalculating the color
            for (let i=0; i<links.length; i++) {
                links[i].value /= links[i].n;
                let bytesAsMegabits = links[i].value / 1000000;
                let percent = Math.min(100, (bytesAsMegabits / links[i].maxBytes) * 100);
                let capacityColor = isColorBlindMode()
                    ? lerpViridis(percent / 100)
                    : lerpGreenToRedViaOrange(100 - percent, 100);
                links[i].lineStyle.color = capacityColor;
            }

            // Filter links with <1 Mbps average throughput
            links = links.filter(link => link.value >= 1000000);

            // Collect node names that are still referenced by links
            const referenced = new Set();
            links.forEach(link => {
                referenced.add(link.source);
                referenced.add(link.target);
            });

            // Always keep the root node
            let rootName = nodes.length > 0 ? nodes[0].name : null;
            if (rootName) referenced.add(rootName);

            // Filter nodes to only those referenced
            nodes = nodes.filter(node => referenced.has(node.name));

            // Update the graph
            graph.update(nodes, links);
        });
        wsClient.send({ NetworkTree: {} });
    }
}

var allNodes = [];
let rootId = 0;
let lastRtt = {};
let paused = false;

function idOfNode(name) {
    for (let i=0; i<allNodes.length; i++) {
        if (allNodes[i][1].name === name) {
            return i;
        }
    }
    return 0;
}

class AllTreeSankey extends DashboardGraph {
    constructor(id) {
        super(id);
        this.model = new AllTreeSankeyGraph();
        this.option = {
            series: [
                {
                    nodeAlign: 'left',
                    type: 'sankey',
                    labelLayout: {
                        moveOverlap: 'shiftx',
                    },
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        this.chart.showLoading();
        this.chart.on('click', (params) => {
            console.log(params.name);
            console.log(this.nodeMap);
            let name = params.name;
            // If it contains a >, it's a link
            if (name.indexOf(" > ") === -1) {
                rootId = idOfNode(name);
            } else {
                rootId = idOfNode(params.data.source);
            }
        });
        $("#btnRoot").click(() => { rootId = 0; });
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

function start() {
    graph.model.onTick(graph);
    setTimeout(start, 1000);
}

function getMaxDepth() {
    let maxDepth = 10;
    let storedDepth = localStorage.getItem("atsDepth");
    if (storedDepth !== null) {
        maxDepth = parseInt(storedDepth);
    } else {
        localStorage.setItem("atsDepth", maxDepth.toString());
    }
    return maxDepth;
}

function bindMaxDepth() {
    let d = document.getElementById("maxDepth");
    d.value = maxDepth;
    d.onclick = () => {
        maxDepth = parseInt(d.value);
        localStorage.setItem("atsDepth", maxDepth.toString());
    };
}

let maxDepth = getMaxDepth();
bindMaxDepth();
let graph = new AllTreeSankey("sankey");

$("#btnPause").click(() => {
    paused = !paused;
    if (paused) {
        $("#btnPause").html("<i class='fa fa-play'></i> Resume");
    } else {
        $("#btnPause").html("<i class='fa fa-pause'></i>Pause");
    }
});

start();
