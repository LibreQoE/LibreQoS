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

const REQUEST_TIMEOUT_MS = 2000;
const MIN_LINK_BITS_PER_SEC = 1_000_000;

let sankeyOverlay = null;

function listenOnceWithTimeout(eventName, timeoutMs, handler, onTimeout) {
    let done = false;
    const wrapped = (msg) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    const timer = setTimeout(() => {
        if (done) return;
        done = true;
        wsClient.off(eventName, wrapped);
        onTimeout();
    }, timeoutMs);
    wsClient.on(eventName, wrapped);
    return { cancel: () => { wsClient.off(eventName, wrapped); clearTimeout(timer); } };
}

function makeOverlay(container, id) {
    container.style.position = "relative";

    const overlay = document.createElement("div");
    overlay.id = id;
    overlay.style.position = "absolute";
    overlay.style.inset = "0";
    overlay.style.display = "none";
    overlay.style.alignItems = "center";
    overlay.style.justifyContent = "center";
    overlay.style.pointerEvents = "none";
    overlay.style.zIndex = "10";
    overlay.style.padding = "16px";

    const panel = document.createElement("div");
    panel.style.background = "var(--lqos-surface)";
    panel.style.border = "1px solid var(--lqos-border)";
    panel.style.borderRadius = "var(--lqos-radius-lg)";
    panel.style.boxShadow = "var(--lqos-shadow-sm)";
    panel.style.padding = "14px 18px";
    panel.style.maxWidth = "560px";
    panel.style.textAlign = "center";
    panel.style.backdropFilter = "blur(10px)";
    panel.style.webkitBackdropFilter = "blur(10px)";

    const title = document.createElement("div");
    title.style.fontWeight = "700";
    title.style.fontSize = "1.1rem";

    const subtitle = document.createElement("div");
    subtitle.className = "text-muted";
    subtitle.style.marginTop = "4px";

    panel.appendChild(title);
    panel.appendChild(subtitle);
    overlay.appendChild(panel);
    container.appendChild(overlay);

    return {
        show: (t, s) => {
            title.textContent = t;
            subtitle.textContent = s || "";
            overlay.style.display = "flex";
        },
        hide: () => {
            overlay.style.display = "none";
        },
    };
}

class AllTreeSankeyGraph extends GenericRingBuffer {
    constructor() {
        super(10);
        this.pending = false;
        this.pendingCancel = null;
    }

    cancelPending() {
        if (this.pendingCancel) {
            try {
                this.pendingCancel.cancel();
            } catch (e) {
                // ignore
            }
        }
        this.pendingCancel = null;
        this.pending = false;
    }

    onTick(graph) {
        if (this.pending) {
            return;
        }
        this.pending = true;

        const self = this;
        this.pendingCancel = listenOnceWithTimeout("NetworkTree", REQUEST_TIMEOUT_MS, (msg) => {
            self.pending = false;
            self.pendingCancel = null;

            if (paused) {
                return;
            }

            const data = msg && msg.data ? msg.data : [];
            // Maintain a 10-second ringbuffer of recent data
            self.push(data);

            let redact = isRedacted();
            let nodes = [];
            let links = [];

            // Build the basic tree from the current head, to ensure
            // that we're displaying the most recent nodes.
            let head = self.getHead();
            if (!Array.isArray(head) || head.length === 0 || !head[rootId] || !head[rootId][1]) {
                if (sankeyOverlay) {
                    sankeyOverlay.show("Limited throughput", "Nothing to render yet.");
                }
                graph.update([], []);
                return;
            }

            allNodes = head;

            const rootParents = head[rootId][1].parents || [];
            let startDepth = rootParents.length - 1;

            for (let i=0; i<head.length; i++) {
                if (!head[i] || !head[i][1]) continue;

                const parents = head[i][1].parents || [];
                let depth = parents.length - startDepth;
                if (depth > maxDepth) {
                    continue;
                }
                // If head[i][1].parents does not contain rootId, skip
                if (rootId !== 0 && !parents.includes(rootId)) {
                    continue;
                }

                const downBytesPerSec = toNumber(head[i][1].current_throughput?.[0], 0);
                const downBitsPerSec = downBytesPerSec * 8;
                const maxBitsPerSec = toNumber(head[i][1].max_throughput?.[0], 0) * 1_000_000;
                const percent = Math.min(100, maxBitsPerSec > 0 ? (downBitsPerSec / maxBitsPerSec) * 100 : 0);

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
                    if (immediateParent === null || immediateParent === undefined) continue;
                    if (!head[immediateParent] || !head[immediateParent][1]) continue;
                    links.push({
                        source: head[immediateParent][1].name,
                        target: head[i][1].name,
                        value: downBitsPerSec,
                        lineStyle: {
                            color: capacityColor,
                        },
                        maxBitsPerSec: maxBitsPerSec,
                        n: 1,
                    });
                }
            }

            // Now we iterate over the entire ringbuffer to accumulate data over the period
            // of the ringbuffer.
            self.iterate((data) => {
                for (let i=0; i<data.length; i++) {
                    if (!data[i] || !data[i][1]) continue;
                    // Search for links that match so we can update the value
                    if (i > 0) {
                        let immediateParent = data[i][1].immediate_parent;
                        if (immediateParent === null || immediateParent === undefined) continue;
                        if (!data[immediateParent] || !data[immediateParent][1]) continue;

                        let link = links.find((link) => {
                            return link.source === data[immediateParent][1].name && link.target === data[i][1].name;
                        });
                        if (link !== undefined) {
                            link.value += toNumber(data[i][1].current_throughput?.[0], 0) * 8;
                            link.n++;
                        }
                    }
                }
            });

            // Now go through the links and average the values, recalculating the color
            for (let i=0; i<links.length; i++) {
                links[i].value /= links[i].n;
                const maxBits = toNumber(links[i].maxBitsPerSec, 0);
                const percent = Math.min(100, maxBits > 0 ? (links[i].value / maxBits) * 100 : 0);
                let capacityColor = isColorBlindMode()
                    ? lerpViridis(percent / 100)
                    : lerpGreenToRedViaOrange(100 - percent, 100);
                links[i].lineStyle.color = capacityColor;
            }

            // Filter links with <1 Mbps average throughput
            links = links.filter(link => link.value >= MIN_LINK_BITS_PER_SEC);

            if (links.length === 0) {
                if (sankeyOverlay) {
                    sankeyOverlay.show("Limited throughput", "Nothing to render yet.");
                }
                graph.update([], []);
                return;
            }

            if (sankeyOverlay) {
                sankeyOverlay.hide();
            }

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
        }, () => {
            self.pending = false;
            self.pendingCancel = null;

            if (paused) {
                return;
            }
            if (sankeyOverlay) {
                sankeyOverlay.show("Waiting for data", "No NetworkTree websocket response received yet.");
            }
            graph.update([], []);
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
        // Replace instead of merge to avoid unbounded Sankey node/link accumulation in ECharts.
        this.chart.setOption(this.option, true);
    }
}

function start() {
    if (!paused) {
        graph.model.onTick(graph);
    }
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
sankeyOverlay = makeOverlay(graph.dom, "allTreeSankeyOverlay");
sankeyOverlay.show("Waiting for data", "Requesting network tree...");

$("#btnPause").click(() => {
    paused = !paused;
    if (paused) {
        $("#btnPause").html("<i class='fa fa-play'></i> Resume");
        graph.model.cancelPending();
    } else {
        $("#btnPause").html("<i class='fa fa-pause'></i>Pause");
        graph.model.onTick(graph);
    }
});

start();
