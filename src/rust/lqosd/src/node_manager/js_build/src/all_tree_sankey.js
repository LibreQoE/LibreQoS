import { DashboardGraph } from "./graphs/dashboard_graph";
import {lerpGreenToRedViaOrange} from "./helpers/scaling";
import {isColorBlindMode} from "./helpers/colorblind";
import {toNumber} from "./lq_js_common/helpers/scaling";
/**
 * Viridis color scale interpolation (0-1 input).
 * Returns hex color string.
 */
function lerpViridis(position) {
    // Viridis colormap sampled at 6 points, interpolated linearly
    const stops = [
        [68, 1, 84],    // #440154
        [59, 82, 139],  // #3B528B
        [33, 145, 140], // #21918C
        [94, 201, 98],  // #5EC962
        [253, 231, 37]  // #FDE725
    ];
    if (position <= 0) return "#440154";
    if (position >= 1) return "#FDE725";
    let idx = position * (stops.length - 1);
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
import {t} from "./i18n/index.js";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();

const REQUEST_TIMEOUT_MS = 2000;
const MIN_LINK_BITS_PER_SEC = 1_000_000;

let sankeyOverlay = null;

function setRootNodeLabel(name) {
    const target = document.getElementById("rootNode");
    if (!target) {
        return;
    }
    target.textContent = name || t("graph.sankey.root");
    target.classList.add("redactable");
}

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

    // translatable: false — "var(--lqos-surface)" "1px solid var(--lqos-border)" "var(--lqos-radius-lg)" "var(--lqos-shadow-sm)" "14px 18px" "blur(10px)" — the overlay panel's CSS values, not labels
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
        show: (text, subtext) => {
            title.textContent = text;
            subtitle.textContent = subtext || "";
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

    render(graph) {
        const head = this.getHead();
        if (!Array.isArray(head) || head.length === 0) {
            setRootNodeLabel(t("graph.sankey.root"));
            if (sankeyOverlay) {
                sankeyOverlay.show(t("sankey.overlay.limited_title"), t("sankey.overlay.nothing"));
            }
            graph.update([], []);
            return;
        }

        if (!head[rootId] || !head[rootId][1]) {
            rootId = 0;
        }

        const rootEntry = head[rootId];
        if (!rootEntry || !rootEntry[1]) {
            setRootNodeLabel(t("graph.sankey.root"));
            if (sankeyOverlay) {
                sankeyOverlay.show(t("sankey.overlay.limited_title"), t("sankey.overlay.nothing"));
            }
            graph.update([], []);
            return;
        }

        allNodes = head;
        setRootNodeLabel(rootEntry[1].name);

        let redact = isRedacted();
        let nodes = [];
        let links = [];
        const linkMap = new Map();
        const rootParents = rootEntry[1].parents || [];
        let startDepth = Math.max(0, rootParents.length - 1);

        for (let i=0; i<head.length; i++) {
            if (!head[i] || !head[i][1]) continue;

            const parents = head[i][1].parents || [];
            let depth = parents.length - startDepth;
            if (depth > maxDepth) {
                continue;
            }
            if (rootId !== 0 && i !== rootId && !parents.includes(rootId)) {
                continue;
            }

            const downBytesPerSec = toNumber(head[i][1].current_throughput?.[0], 0);
            const downBitsPerSec = downBytesPerSec * 8;
            const maxBitsPerSec = toNumber(head[i][1].max_throughput?.[0], 0) * 1_000_000;
            const percent = Math.min(100, maxBitsPerSec > 0 ? (downBitsPerSec / maxBitsPerSec) * 100 : 0);

            let capacityColor = isColorBlindMode()
                ? lerpViridis(percent / 100)
                : lerpGreenToRedViaOrange(100 - percent, 100);

            let label = {
                fontSize: 10,
                color: "#999",
                formatter: (params) => {
                    // translatable: false — "(Generated Site)" is the server's synthetic-node name suffix, stripped here and never shown
                    return trimStringWithElipsis(params.name.replace("(Generated Site) ", ""), 14);
                }
            };
            if (redact) {
                // translatable: false — "Illegible" is a font family the redaction mode applies, not text on screen
                label.fontFamily = "Illegible";
            }

            nodes.push({
                name: head[i][1].name,
                label: label,
                itemStyle: {
                    color: capacityColor
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
                linkMap.set(
                    `${head[immediateParent][1].name}\u0000${head[i][1].name}`,
                    links[links.length - 1],
                );
            }
        }

        this.iterate((data) => {
            for (let i=0; i<data.length; i++) {
                if (!data[i] || !data[i][1]) continue;
                if (i > 0) {
                    let immediateParent = data[i][1].immediate_parent;
                    if (immediateParent === null || immediateParent === undefined) continue;
                    if (!data[immediateParent] || !data[immediateParent][1]) continue;

                    const link = linkMap.get(
                        `${data[immediateParent][1].name}\u0000${data[i][1].name}`,
                    );
                    if (link !== undefined) {
                        link.value += toNumber(data[i][1].current_throughput?.[0], 0) * 8;
                        link.n++;
                    }
                }
            }
        });

        for (let i=0; i<links.length; i++) {
            links[i].value /= links[i].n;
            const maxBits = toNumber(links[i].maxBitsPerSec, 0);
            const percent = Math.min(100, maxBits > 0 ? (links[i].value / maxBits) * 100 : 0);
            let capacityColor = isColorBlindMode()
                ? lerpViridis(percent / 100)
                : lerpGreenToRedViaOrange(100 - percent, 100);
            links[i].lineStyle.color = capacityColor;
        }

        links = links.filter(link => link.value >= MIN_LINK_BITS_PER_SEC);

        if (links.length === 0) {
            if (sankeyOverlay) {
                sankeyOverlay.show(t("sankey.overlay.limited_title"), t("sankey.overlay.nothing"));
            }
            graph.update([], []);
            return;
        }

        if (sankeyOverlay) {
            sankeyOverlay.hide();
        }

        const referenced = new Set();
        links.forEach(link => {
            referenced.add(link.source);
            referenced.add(link.target);
        });

        referenced.add(rootEntry[1].name);
        nodes = nodes.filter(node => referenced.has(node.name));
        graph.update(nodes, links);
    }

    onTick(graph) {
        if (this.pending) {
            return;
        }
        this.pending = true;

        const self = this;
        this.pendingCancel = listenOnceWithTimeout("NetworkTreeLite", REQUEST_TIMEOUT_MS, (msg) => {
            self.pending = false;
            self.pendingCancel = null;

            if (paused) {
                return;
            }

            const data = msg && msg.data ? msg.data : [];
            self.push(data);
            self.render(graph);
        }, () => {
            self.pending = false;
            self.pendingCancel = null;

            if (paused) {
                return;
            }
            if (sankeyOverlay) {
                sankeyOverlay.show(t("common.state.waiting"), t("sankey.overlay.no_response"));
            }
            graph.update([], []);
        });

        wsClient.send({ NetworkTreeLite: {} });
    }

    rerender(graph) {
        this.render(graph);
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
            let name = params.name;
            if (name.indexOf(" > ") === -1) {
                rootId = idOfNode(name);
            } else {
                rootId = idOfNode(params.data.source);
            }
            this.model.rerender(this);
        });
        $("#btnRoot").click(() => {
            rootId = 0;
            this.model.rerender(this);
        });
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

function start() {
    if (!paused) {
        graph.model.onTick(graph);
    }
    loopTimer = setTimeout(() => {
        loopTimer = null;
        start();
    }, 1000);
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
    d.addEventListener("change", () => {
        maxDepth = parseInt(d.value);
        localStorage.setItem("atsDepth", maxDepth.toString());
        graph.model.rerender(graph);
    });
}

let maxDepth = getMaxDepth();
let graph = new AllTreeSankey("sankey");
bindMaxDepth();
sankeyOverlay = makeOverlay(graph.dom, "allTreeSankeyOverlay");
sankeyOverlay.show(t("common.state.waiting"), t("sankey.overlay.requesting"));
let loopTimer = null;

$("#btnPause").click(() => {
    paused = !paused;
    if (paused) {
        $("#btnPause").html(`<i class='fa fa-play'></i> ${t("dashboard.resume")}`);
        graph.model.cancelPending();
        if (loopTimer) {
            clearTimeout(loopTimer);
            loopTimer = null;
        }
    } else {
        $("#btnPause").html(`<i class='fa fa-pause'></i>${t("dashboard.pause")}`);
        start();
    }
});

document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
        graph.model.cancelPending();
        if (loopTimer) {
            clearTimeout(loopTimer);
            loopTimer = null;
        }
        return;
    }
    if (!paused && !loopTimer) {
        start();
    }
});

window.addEventListener("beforeunload", () => {
    graph.model.cancelPending();
    if (loopTimer) {
        clearTimeout(loopTimer);
        loopTimer = null;
    }
});

start();
