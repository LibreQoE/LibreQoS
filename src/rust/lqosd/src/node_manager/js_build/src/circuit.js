// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";
import {clearDiv, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {formatRetransmit, formatRtt, formatThroughput, lerpGreenToRedViaOrange, formatMbps} from "./helpers/scaling";
import {colorByQoqScore, colorByRttMs} from "./helpers/color_scales";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {QooScoreGauge} from "./graphs/qoo_score_gauge";
import {CircuitTotalGraph} from "./graphs/circuit_throughput_graph";
import {CircuitRetransmitGraph} from "./graphs/circuit_retransmit_graph";
import {scaleNanos, scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {openFlowRttExcludeWizard} from "./lq_js_common/helpers/flow_rtt_exclude_wizard";
import {DevicePingHistogram} from "./graphs/device_ping_graph";
import {WindowedLatencyHistogram} from "./graphs/windowed_latency_histogram";
import {FlowsSankey, getRenderableSankeyFlowCount} from "./graphs/flow_sankey";
import {get_ws_client, subscribeWS} from "./pubsub/ws";
import {CakeBacklog} from "./graphs/cake_backlog";
import {CakeDelays} from "./graphs/cake_delays";
import {CakeQueueLength} from "./graphs/cake_queue_length";
import {CakeTraffic} from "./graphs/cake_traffic";
import {CakeMarks} from "./graphs/cake_marks";
import {CakeDrops} from "./graphs/cake_drops";
import {QueuingActivityWaveform} from "./graphs/queuing_activity_waveform";
import {getNodeIdMap, linkToTreeNode} from "./executive_utils";
import {loadConfig} from "./config/config_helper";

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let circuit_id = decodeURI(params.id);
let plan = null;
let channelLink = null;
let cakeChannel = null;
let pinger = null;
let flowChannel = null;
let funnelSubscription = null;
let speedometer = null;
let qooGauge = null;
let totalThroughput = null;
let totalRetransmits = null;
let deviceGraphs = {};
let devicePings = [];
let flowSankey = null;
let funnelGraphs = {};
let funnelParents = [];
let funnelParentSignature = [];
let funnelInitialized = false;
let funnelParentNodeName = null;
let excludeRttToggle = null;
let excludeRttLastValue = false;
let excludeRttBusy = false;
let latestFlowMsg = null;
let latestCakeMsg = null;
let cakeGraphs = null;
let cakeQueueUnavailable = false;
let circuitSqmOverride = "";
let queuingActivityGraph = null;
let latestCircuitDevices = [];
let latestCircuitQooScore = null;
let queuingActivityDirection = "down";
let deviceGraphSpecs = [];
let deviceGraphsInitialized = false;
const QUEUING_ACTIVITY_RTT_FLOOR_BPS = 200_000;
const DEFAULT_RTT_THRESHOLDS = { green_ms: 0, yellow_ms: 100, red_ms: 200 };
let currentRttThresholds = { ...DEFAULT_RTT_THRESHOLDS };
const wsClient = get_ws_client();
const RECENT_TRAFFIC_FLOW_WINDOW_NANOS = 30_000_000_000;
const TRAFFIC_FLOW_HIDE_THRESHOLD_BPS = 1024 * 1024;
const DEFAULT_TRAFFIC_PAGE_SIZE = 100;

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function isElementVisible(el) {
    return !!(el && el.offsetWidth > 0 && el.offsetHeight > 0);
}

function hasRenderableSize(el) {
    if (!el) {
        return false;
    }
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
}

function runWhenRenderable(el, callback, attempts = 10) {
    if (!el) {
        return;
    }
    if (hasRenderableSize(el)) {
        callback();
        return;
    }
    if (attempts <= 0) {
        return;
    }
    window.setTimeout(() => {
        runWhenRenderable(el, callback, attempts - 1);
    }, 50);
}

function loadingBlockHtml(label, sizeClass = "") {
    const size = sizeClass ? ` ${sizeClass}` : "";
    return `<div class="lqos-loading-block${size}"><i class="fa fa-spinner fa-spin"></i><span>${label}</span></div>`;
}

function initTooltipsWithin(rootEl = document) {
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) {
        return;
    }
    const elements = rootEl.querySelectorAll('[data-bs-toggle="tooltip"]');
    elements.forEach((element) => {
        if (bootstrap.Tooltip.getOrCreateInstance) {
            bootstrap.Tooltip.getOrCreateInstance(element);
        } else {
            new bootstrap.Tooltip(element);
        }
    });
}

function applyParentNodeLink(parentNodeName) {
    const parentNodeEl = document.getElementById("parentNode");
    if (!parentNodeEl) {
        return;
    }

    parentNodeEl.textContent = parentNodeName || "";

    if (!parentNodeName) {
        parentNodeEl.removeAttribute("href");
        parentNodeEl.removeAttribute("title");
        parentNodeEl.style.pointerEvents = "none";
        return;
    }

    parentNodeEl.style.pointerEvents = "";
    getNodeIdMap().then((nodeIdLookup) => {
        const href = linkToTreeNode(parentNodeName, nodeIdLookup);
        if (!href) {
            parentNodeEl.removeAttribute("href");
            parentNodeEl.removeAttribute("title");
            parentNodeEl.style.pointerEvents = "none";
            return;
        }
        parentNodeEl.href = href;
        parentNodeEl.title = `Open ${parentNodeName} in Tree`;
    });
}

function resizeGraphIfVisible(graph) {
    if (!graph || !graph.chart || typeof graph.chart.resize !== "function") {
        return;
    }
    const dom = typeof graph.chart.getDom === "function" ? graph.chart.getDom() : graph.dom;
    if (!isElementVisible(dom)) {
        return;
    }
    graph.chart.resize();
}

function formatBitsPerSecondLabel(bitsPerSecond) {
    return `${scaleNumber(bitsPerSecond, 1)}bps`;
}

function medianOfSorted(values) {
    if (!Array.isArray(values) || values.length === 0) {
        return null;
    }
    const middle = Math.floor(values.length / 2);
    if (values.length % 2 === 1) {
        return values[middle];
    }
    return (values[middle - 1] + values[middle]) / 2;
}

function weightedMedian(entries) {
    if (!Array.isArray(entries) || entries.length === 0) {
        return null;
    }

    const totalWeight = entries.reduce((sum, entry) => sum + Math.max(0, toNumber(entry.weight, 0)), 0);
    if (!(totalWeight > 0)) {
        return null;
    }

    const threshold = totalWeight / 2;
    let running = 0;
    for (const entry of entries) {
        running += Math.max(0, toNumber(entry.weight, 0));
        if (running >= threshold) {
            return toNumber(entry.value, null);
        }
    }

    return toNumber(entries[entries.length - 1].value, null);
}

function currentCircuitRttP50Ms(direction) {
    const directional = direction === "up" ? "up" : "down";
    const weightedEntries = [];
    const fallbackValues = [];

    latestCircuitDevices.forEach((device) => {
        const currentP50Nanos = toNumber(device?.rtt_current_p50_nanos?.[directional], 0);
        if (!(currentP50Nanos > 0)) {
            return;
        }

        const throughputBps = toNumber(device?.bytes_per_second?.[directional], 0) * 8;
        const currentP50Ms = currentP50Nanos / 1_000_000.0;
        fallbackValues.push(currentP50Ms);

        if (throughputBps > QUEUING_ACTIVITY_RTT_FLOOR_BPS) {
            weightedEntries.push({
                value: currentP50Ms,
                weight: throughputBps,
            });
        }
    });

    weightedEntries.sort((a, b) => a.value - b.value);
    const weighted = weightedMedian(weightedEntries);
    if (Number.isFinite(weighted)) {
        return weighted;
    }

    fallbackValues.sort((a, b) => a - b);
    const fallbackMedian = medianOfSorted(fallbackValues);
    return Number.isFinite(fallbackMedian) ? fallbackMedian : null;
}

function formatCircuitRttLabel(rttMs) {
    const value = toNumber(rttMs, NaN);
    if (!Number.isFinite(value) || value <= 0) {
        return "-";
    }
    return `${value.toFixed(value >= 10 ? 0 : 1)} ms`;
}

function normalizeRttThresholds(rawThresholds) {
    const green = Math.max(0, Math.round(toNumber(rawThresholds?.green_ms ?? rawThresholds?.greenMs, DEFAULT_RTT_THRESHOLDS.green_ms)));
    const yellow = Math.max(green, Math.round(toNumber(rawThresholds?.yellow_ms ?? rawThresholds?.yellowMs, DEFAULT_RTT_THRESHOLDS.yellow_ms)));
    const red = Math.max(yellow, 1, Math.round(toNumber(rawThresholds?.red_ms ?? rawThresholds?.redMs, DEFAULT_RTT_THRESHOLDS.red_ms)));
    return {
        green_ms: green,
        yellow_ms: yellow,
        red_ms: red,
    };
}

function applyRttThresholds(rawThresholds) {
    currentRttThresholds = normalizeRttThresholds(rawThresholds);
    if (queuingActivityGraph) {
        queuingActivityGraph.setRttThresholds(currentRttThresholds);
    }
    updateQueuingActivityCards();
}

function loadRttThresholds() {
    loadConfig(
        () => {
            applyRttThresholds(window.config?.rtt_thresholds);
        },
        () => {
            applyRttThresholds(null);
        },
    );
}

function currentActiveFlowCount() {
    return latestTrafficRows.length;
}

function currentDirectionValue(pair, direction, fallback = 0) {
    return toNumber(pair?.[direction], fallback);
}

function currentQueuingActivitySnapshot() {
    const throughputBps = latestCircuitDevices.reduce((sum, device) => {
        return sum + (currentDirectionValue(device?.bytes_per_second, queuingActivityDirection, 0) * 8);
    }, 0);
    const ceilingMbps = currentDirectionValue(plan, queuingActivityDirection, 0);
    const ceilingBps = ceilingMbps * 1_000_000.0;
    const atCeiling = ceilingBps > 0 && throughputBps >= (ceilingBps * 0.95);
    const utilizationPercent = ceilingBps > 0
        ? Math.max(0, Math.min(999, (throughputBps / ceilingBps) * 100))
        : 0;
    return {
        throughputBps,
        ceilingBps,
        rttP50Ms: currentCircuitRttP50Ms(queuingActivityDirection),
        activeFlows: currentActiveFlowCount(),
        utilizationPercent,
        atCeiling,
    };
}

function updateQueuingActivityCards() {
    const throughputEl = document.getElementById("queuingActivityThroughput");
    const rttEl = document.getElementById("queuingActivityRtt");
    const flowsEl = document.getElementById("queuingActivityFlows");
    const utilizationEl = document.getElementById("queuingActivityUtilization");
    const ceilingLegendEl = document.getElementById("queuingActivityLegendCeiling");
    if (!throughputEl || !rttEl || !flowsEl || !utilizationEl) {
        return;
    }

    const snapshot = currentQueuingActivitySnapshot();
    throughputEl.textContent = formatBitsPerSecondLabel(snapshot.throughputBps);
    rttEl.textContent = formatCircuitRttLabel(snapshot.rttP50Ms);
    rttEl.style.color = snapshot.rttP50Ms !== null ? colorByRttMs(snapshot.rttP50Ms, currentRttThresholds) : "";
    flowsEl.textContent = String(snapshot.activeFlows);
    utilizationEl.textContent = `${snapshot.utilizationPercent.toFixed(0)}%`;
    utilizationEl.classList.toggle("is-active", snapshot.atCeiling);
    if (ceilingLegendEl) {
        ceilingLegendEl.classList.toggle("is-active", snapshot.atCeiling);
    }
}

function pushQueuingActivitySample() {
    if (!queuingActivityGraph || !latestCircuitDevices.length || !plan) {
        updateQueuingActivityCards();
        return;
    }

    const downThroughputBps = latestCircuitDevices.reduce((sum, device) => {
        return sum + (currentDirectionValue(device?.bytes_per_second, "down", 0) * 8);
    }, 0);
    const upThroughputBps = latestCircuitDevices.reduce((sum, device) => {
        return sum + (currentDirectionValue(device?.bytes_per_second, "up", 0) * 8);
    }, 0);

    queuingActivityGraph.pushSample({
        timestamp: Date.now(),
        throughputBps: {
            down: downThroughputBps,
            up: upThroughputBps,
        },
        ceilingBps: {
            down: currentDirectionValue(plan, "down", 0) * 1_000_000.0,
            up: currentDirectionValue(plan, "up", 0) * 1_000_000.0,
        },
        rttP50Ms: {
            down: currentCircuitRttP50Ms("down"),
            up: currentCircuitRttP50Ms("up"),
        },
    });
    updateQueuingActivityCards();
}

function applyQueuingDirection(direction) {
    queuingActivityDirection = direction === "up" ? "up" : "down";
    if (queuingActivityGraph) {
        queuingActivityGraph.setDirection(queuingActivityDirection);
    }
    updateQueuingActivityCards();
}

function ensureQueuingActivityGraph() {
    const target = document.getElementById("queuingActivityGraph");
    if (!target || queuingActivityGraph || !isElementVisible(target)) {
        return;
    }
    runWhenRenderable(target, () => {
        if (queuingActivityGraph || !hasRenderableSize(target)) {
            return;
        }
        queuingActivityGraph = new QueuingActivityWaveform("queuingActivityGraph");
        queuingActivityGraph.setDirection(queuingActivityDirection);
        queuingActivityGraph.setRttThresholds(currentRttThresholds);
        pushQueuingActivitySample();
    });
}

function initQueuingActivityControls() {
    const downloadToggle = document.getElementById("queuingDirectionDown");
    const uploadToggle = document.getElementById("queuingDirectionUp");
    if (downloadToggle) {
        downloadToggle.addEventListener("change", () => {
            if (downloadToggle.checked) {
                applyQueuingDirection("down");
            }
        });
    }
    if (uploadToggle) {
        uploadToggle.addEventListener("change", () => {
            if (uploadToggle.checked) {
                applyQueuingDirection("up");
            }
        });
    }
    applyQueuingDirection("down");
}

function isTrafficTabActive() {
    return document.getElementById("traffic-tab")?.classList.contains("active") ?? false;
}

function isSankeyTabActive() {
    return document.getElementById("sankey-tab")?.classList.contains("active") ?? false;
}

function applyFlowSankeyMessage(msg) {
    $("#activeFlowCount").text(getRenderableSankeyFlowCount(msg));
    if (!flowSankey || !msg) {
        return;
    }
    flowSankey.update(msg);
    resizeGraphIfVisible(flowSankey);
}

function ensureFlowSankey() {
    const target = document.getElementById("flowSankey");
    if (!target || flowSankey || !isElementVisible(target)) {
        return;
    }
    runWhenRenderable(target, () => {
        if (flowSankey || !hasRenderableSize(target)) {
            return;
        }
        flowSankey = new FlowsSankey("flowSankey");
        applyFlowSankeyMessage(latestFlowMsg);
    });
}

function resizeFunnelGraphs() {
    Object.values(funnelGraphs).forEach((graphSet) => {
        if (!graphSet) return;
        Object.values(graphSet).forEach((graph) => resizeGraphIfVisible(graph));
    });
}

function initializeDeviceGraphs() {
    if (deviceGraphsInitialized) {
        return;
    }
    deviceGraphSpecs.forEach((spec) => {
        if (!document.getElementById(spec.id) || deviceGraphs[spec.id]) {
            return;
        }
        deviceGraphs[spec.id] = spec.factory(spec.id);
    });
    deviceGraphsInitialized = true;
    if (latestCircuitDevices.length > 0) {
        fillLiveDevices(latestCircuitDevices);
        if (speedometer && totalThroughput && totalRetransmits) {
            updateSpeedometer(latestCircuitDevices);
        }
    }
}

function resizeDeviceGraphs() {
    Object.values(deviceGraphs).forEach((graph) => resizeGraphIfVisible(graph));
}

function arrayEquals(a, b) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) {
        return false;
    }
    for (let i = 0; i < a.length; i++) {
        if (a[i] !== b[i]) {
            return false;
        }
    }
    return true;
}

function resolveFunnelState(msg, parentNode) {
    const data = msg && msg.data ? msg.data : [];
    const namedEntry = data.find((node) => node[1] && node[1].name === parentNode);
    if (!namedEntry) {
        return null;
    }

    const immediateParent = namedEntry[1];
    const parentIndexes = Array.isArray(immediateParent.parents) ? [...immediateParent.parents] : [];
    const parentSignature = parentIndexes.map((parent) => {
        const node = data[parent] && data[parent][1] ? data[parent][1] : null;
        if (!node) {
            return `${parent}:missing`;
        }
        return `${parent}:${node.name}:${node.is_virtual === true ? "virtual" : "physical"}`;
    });

    return {
        data,
        immediateParent,
        parentIndexes,
        parentSignature,
    };
}

function buildFunnelPathCard(state, displayParents) {
    const pathCard = document.createElement("div");
    pathCard.classList.add("lqos-funnel-path-card");

    const topRow = document.createElement("div");
    topRow.classList.add("lqos-funnel-path-top");

    const title = document.createElement("div");
    title.classList.add("lqos-funnel-path-title");
    title.textContent = "Queue Path";
    topRow.appendChild(title);

    const meta = document.createElement("div");
    meta.classList.add("lqos-funnel-path-meta");

    const parentPill = document.createElement("span");
    parentPill.classList.add("lqos-funnel-path-pill");
    parentPill.textContent = "Circuit parent";
    meta.appendChild(parentPill);

    const countPill = document.createElement("span");
    countPill.classList.add("lqos-funnel-path-pill");
    countPill.textContent = `${displayParents.length} upstream node${displayParents.length === 1 ? "" : "s"}`;
    meta.appendChild(countPill);

    topRow.appendChild(meta);
    pathCard.appendChild(topRow);

    const chain = document.createElement("div");
    chain.classList.add("lqos-funnel-path-chain");

    const origin = document.createElement("span");
    origin.classList.add("lqos-funnel-path-node", "is-origin", "redactable");
    origin.textContent = state.immediateParent.name || "Unknown";
    chain.appendChild(origin);

    displayParents.forEach(({ node }) => {
        const separator = document.createElement("span");
        separator.classList.add("lqos-funnel-path-separator");
        separator.textContent = "→";
        chain.appendChild(separator);

        const ancestor = document.createElement("span");
        ancestor.classList.add("lqos-funnel-path-node", "redactable");
        ancestor.textContent = node.name || "Unknown";
        chain.appendChild(ancestor);
    });

    pathCard.appendChild(chain);

    const note = document.createElement("p");
    note.classList.add("text-muted", "small", "mb-0");
    note.textContent =
        displayParents.length > 0
            ? "Live upstream queue ancestors for this circuit, shown with the same order used below."
            : "This circuit parent does not currently have additional upstream queue ancestors.";
    pathCard.appendChild(note);

    return pathCard;
}

function buildFunnelEmptyState(message) {
    const empty = document.createElement("div");
    empty.classList.add("lqos-funnel-empty-state");

    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-circle-info");
    empty.appendChild(icon);

    const text = document.createElement("span");
    text.textContent = message;
    empty.appendChild(text);

    return empty;
}

function renderFunnel(state) {
    const target = document.getElementById("theFunnel");
    if (!target) {
        return;
    }

    if (!state) {
        funnelGraphs = {};
        funnelParents = [];
        funnelParentSignature = [];
        clearDiv(target);
        target.appendChild(buildFunnelEmptyState("No parent node found for this circuit."));
        return;
    }

    const displayParents = [...state.parentIndexes]
        .reverse()
        .map((parent, index) => {
            const node = state.data[parent] && state.data[parent][1] ? state.data[parent][1] : null;
            if (!node) {
                return null;
            }
            return {
                parent,
                node,
                position: index + 1,
            };
        })
        .filter(Boolean);

    const parentDiv = document.createElement("div");
    parentDiv.classList.add("lqos-funnel-stack");
    parentDiv.appendChild(buildFunnelPathCard(state, displayParents));

    if (displayParents.length === 0) {
        parentDiv.appendChild(buildFunnelEmptyState("No upstream queue ancestors are currently available beyond the circuit parent."));
    }

    displayParents.forEach(({ parent, node, position }) => {
        const card = document.createElement("section");
        card.classList.add("lqos-funnel-node-card");

        const header = document.createElement("div");
        header.classList.add("lqos-funnel-node-header");

        const titleWrap = document.createElement("div");
        titleWrap.classList.add("lqos-funnel-node-title");

        const heading = document.createElement("h5");
        const icon = document.createElement("i");
        icon.classList.add("fa", "fa-sitemap");
        heading.appendChild(icon);

        const name = document.createElement("span");
        name.classList.add("redactable");
        name.textContent = node.name || "Unknown";
        heading.appendChild(name);
        titleWrap.appendChild(heading);

        const subtitle = document.createElement("p");
        subtitle.classList.add("text-muted", "small", "mb-0");
        subtitle.textContent = "Live queue telemetry for this upstream node.";
        titleWrap.appendChild(subtitle);

        header.appendChild(titleWrap);

        const badges = document.createElement("div");
        badges.classList.add("lqos-funnel-node-badges");

        const typeBadge = document.createElement("span");
        typeBadge.classList.add("lqos-funnel-node-type");
        if (node.is_virtual === true) {
            typeBadge.classList.add("is-virtual");
        }
        typeBadge.textContent = node.is_virtual === true ? "Virtual" : "Physical";
        badges.appendChild(typeBadge);

        const stepBadge = document.createElement("span");
        stepBadge.classList.add("lqos-funnel-node-step");
        stepBadge.textContent = `Ancestor ${position} / ${displayParents.length}`;
        badges.appendChild(stepBadge);

        header.appendChild(badges);
        card.appendChild(header);

        const row = document.createElement("div");
        row.classList.add("row", "g-3");

        const chartSpecs = [
            { id: "funnel_tp_" + parent, height: "250px" },
            { id: "funnel_rxmit_" + parent, height: "250px" },
            { id: "funnel_rtt_" + parent, height: "250px" },
        ];

        chartSpecs.forEach((chart) => {
            const col = document.createElement("div");
            col.classList.add("col-12", "col-xl-4");

            const panel = document.createElement("div");
            panel.classList.add("lqos-funnel-chart-panel");

            const graph = document.createElement("div");
            graph.id = chart.id;
            graph.style.height = chart.height;
            panel.appendChild(graph);
            col.appendChild(panel);
            row.appendChild(col);
        });

        card.appendChild(row);
        parentDiv.appendChild(card);
    });

    funnelGraphs = {};
    clearDiv(target);
    target.appendChild(parentDiv);

    requestAnimationFrame(() => {
        setTimeout(() => {
            displayParents.forEach(({ parent }) => {
                if (!document.getElementById("funnel_tp_" + parent)) {
                    return;
                }
                let tpGraph = new CircuitTotalGraph("funnel_tp_" + parent, "Throughput");
                let rxmitGraph = new CircuitRetransmitGraph("funnel_rxmit_" + parent, "Retransmits");
                let rttGraph = new WindowedLatencyHistogram("funnel_rtt_" + parent, "Latency Histogram", 300000);
                funnelGraphs[parent] = {
                    tp: tpGraph,
                    rxmit: rxmitGraph,
                    rtt: rttGraph,
                };
                resizeGraphIfVisible(tpGraph);
                resizeGraphIfVisible(rxmitGraph);
                resizeGraphIfVisible(rttGraph);
            });
        }, 0);
    });

    funnelParents = state.parentIndexes;
    funnelParentSignature = state.parentSignature;
}

function updateCakeTabAvailability(msg) {
    try {
        const kindDown = (msg?.kind_down || "").toLowerCase();
        const kindUp = (msg?.kind_up || "").toLowerCase();
        const tabBtn = document.getElementById("cake-tab");
        const tabLi = tabBtn ? tabBtn.parentElement : null;
        const tabContent = document.getElementById("cake");

        if (kindDown === "none" && kindUp === "none") {
            cakeQueueUnavailable = true;
            if (tabLi) tabLi.style.display = "none";
            if (tabContent) tabContent.style.display = "none";
            return false;
        }

        cakeQueueUnavailable = false;
        if (tabLi) tabLi.style.display = "";
        if (tabContent) tabContent.style.display = "";

        if (tabBtn) {
            tabBtn.innerHTML = '<i class="fa fa-birthday-cake"></i> Queue Stats';
        }
    } catch (e) {
        // Ignore label updates; data updates still continue.
    }
    return true;
}

function renderCakeGraphShell() {
    const cakeTab = document.getElementById("cake");
    if (!cakeTab || document.getElementById("cakeBacklog")) {
        return;
    }
    cakeTab.innerHTML = `
        <div class="lqos-cake-panel">
            <div class="lqos-cake-header">
                <div class="lqos-cake-header-copy">
                    <h5><i class="fa fa-birthday-cake"></i> Queue Stats</h5>
                    <p class="text-muted small mb-0">Raw 1s scatter samples over the last 3 minutes. Hover any queue chart to inspect the same timestamp across all six charts.</p>
                </div>
                <div class="lqos-cake-meta">
                    <span class="lqos-cake-meta-pill">Last 3 minutes</span>
                    <span class="lqos-cake-meta-pill">1s samples</span>
                    <span class="lqos-cake-meta-pill">Synchronized hover</span>
                </div>
            </div>
            <div class="row g-3">
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel is-primary">
                        <div id="cakeTraffic" style="height: 280px"></div>
                    </div>
                </div>
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel is-primary">
                        <div id="cakeDelays" style="height: 280px"></div>
                    </div>
                </div>
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel is-primary">
                        <div id="cakeBacklog" style="height: 280px"></div>
                    </div>
                </div>
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel">
                        <div id="cakeQueueLength" style="height: 240px"></div>
                    </div>
                </div>
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel">
                        <div id="cakeMarks" style="height: 240px"></div>
                    </div>
                </div>
                <div class="col-12 col-xl-4">
                    <div class="lqos-cake-chart-panel">
                        <div id="cakeDrops" style="height: 240px"></div>
                    </div>
                </div>
                <div class="col-12">
                    <div class="lqos-cake-info-card">
                        <div class="lqos-cake-info-item">
                            <div class="lqos-cake-info-label">Queue Memory</div>
                            <div class="lqos-cake-info-value"><span id="cakeQueueMemory">?</span></div>
                        </div>
                        <div class="lqos-cake-info-item">
                            <div class="lqos-cake-info-label">Live Queue Type</div>
                            <div class="lqos-cake-info-value"><span id="cakeQueueType">?</span></div>
                        </div>
                        <div class="lqos-cake-info-item">
                            <div class="lqos-cake-info-label">Interpretation</div>
                            <div class="lqos-cake-info-value text-muted small">Download plots above zero, upload plots below zero. Tooltips always show absolute values with direction labels.</div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    `;
    setQueueTypeDisplayFromKinds(latestCakeMsg?.kind_down, latestCakeMsg?.kind_up);
}

function applyCakeMessage(msg) {
    if (!msg) {
        return;
    }
    setQueueTypeDisplayFromKinds(msg.kind_down, msg.kind_up);
    if (!cakeGraphs) {
        return;
    }
    $("#cakeQueueMemory").text(scaleNumber(msg.current_download.memory_used) + " / " + scaleNumber(msg.current_upload.memory_used));
    cakeGraphs.backlog.update(msg);
    resizeGraphIfVisible(cakeGraphs.backlog);
    cakeGraphs.delays.update(msg);
    resizeGraphIfVisible(cakeGraphs.delays);
    cakeGraphs.queueLength.update(msg);
    resizeGraphIfVisible(cakeGraphs.queueLength);
    cakeGraphs.traffic.update(msg);
    resizeGraphIfVisible(cakeGraphs.traffic);
    cakeGraphs.marks.update(msg);
    resizeGraphIfVisible(cakeGraphs.marks);
    cakeGraphs.drops.update(msg);
    resizeGraphIfVisible(cakeGraphs.drops);
}

function ensureCakeGraphs() {
    const cakeTab = document.getElementById("cake");
    if (!cakeTab || cakeGraphs || cakeQueueUnavailable || !isElementVisible(cakeTab)) {
        return;
    }
    runWhenRenderable(cakeTab, () => {
        if (cakeGraphs || !hasRenderableSize(cakeTab)) {
            return;
        }
        renderCakeGraphShell();
        cakeGraphs = {
            backlog: new CakeBacklog("cakeBacklog"),
            delays: new CakeDelays("cakeDelays"),
            queueLength: new CakeQueueLength("cakeQueueLength"),
            traffic: new CakeTraffic("cakeTraffic"),
            marks: new CakeMarks("cakeMarks"),
            drops: new CakeDrops("cakeDrops"),
        };
        const cakeChartInstances = Object.values(cakeGraphs)
            .map((graph) => graph?.chart)
            .filter(Boolean);
        cakeChartInstances.forEach((chart) => {
            chart.group = "circuitCakeQueueStats";
        });
        echarts.connect("circuitCakeQueueStats");
        applyCakeMessage(latestCakeMsg);
    });
}

function initTabLifecycle(parentNode) {
    const tabs = document.querySelectorAll('#myTab button[data-bs-toggle="tab"]');
    tabs.forEach((tab) => {
        tab.addEventListener("shown.bs.tab", () => {
            window.requestAnimationFrame(() => {
                const target = tab.getAttribute("data-bs-target");
                if (target === "#queuing") {
                    ensureQueuingActivityGraph();
                    updateQueuingActivityCards();
                    return;
                }
                if (target === "#devs") {
                    initializeDeviceGraphs();
                    resizeDeviceGraphs();
                    return;
                }
                if (target === "#sankey") {
                    ensureFlowSankey();
                    applyFlowSankeyMessage(latestFlowMsg);
                    return;
                }
                if (target === "#traffic") {
                    renderTrafficTab();
                    return;
                }
                if (target === "#funnel") {
                    if (!funnelInitialized) {
                        funnelInitialized = true;
                        initialFunnel(parentNode);
                    } else {
                        resizeFunnelGraphs();
                    }
                    return;
                }
                if (target === "#cake") {
                    ensureCakeGraphs();
                    applyCakeMessage(latestCakeMsg);
                }
            });
        });
    });

    window.requestAnimationFrame(() => {
        ensureQueuingActivityGraph();
        updateQueuingActivityCards();
    });
}

function formatIpBytes(bytes) {
    const list = Array.from(bytes);
    if (list.length === 4) {
        return list.join(".");
    }
    if (list.length === 16) {
        const parts = [];
        for (let i = 0; i < list.length; i += 2) {
            const part = (list[i] << 8) | list[i + 1];
            parts.push(part.toString(16).padStart(4, "0"));
        }
        return parts.join(":");
    }
    return list.join(".");
}

function ipToString(ip) {
    if (typeof ip === "string") {
        return ip;
    }
    if (ip instanceof Uint8Array || Array.isArray(ip)) {
        return formatIpBytes(ip);
    }
    return String(ip);
}

function parseDirectionalSqmToken(token) {
    const raw = (token ?? "").toString().trim().toLowerCase();
    if (!raw) {
        return { down: "", up: "" };
    }
    if (!raw.includes("/")) {
        return { down: raw, up: raw };
    }
    const [down, up] = raw.split("/", 2);
    return {
        down: (down ?? "").toString().trim(),
        up: (up ?? "").toString().trim(),
    };
}

function formatQueueTypeDisplay(sqmToken) {
    const { down, up } = parseDirectionalSqmToken(sqmToken);
    return formatDirectionalQueueTypeDisplay(down, up);
}

function formatDirectionalQueueTypeDisplay(downToken, upToken) {
    const down = (downToken ?? "").toString().trim().toLowerCase();
    const up = (upToken ?? "").toString().trim().toLowerCase();
    const downLabel = down || "Unknown";
    const upLabel = up || down || "Unknown";
    return `${downLabel} / ${upLabel}`;
}

function setQueueTypeDisplay(sqmToken) {
    const queueTypeEl = document.getElementById("cakeQueueType");
    if (!queueTypeEl) {
        return;
    }
    queueTypeEl.textContent = formatQueueTypeDisplay(sqmToken);
}

function setQueueTypeDisplayFromKinds(kindDown, kindUp) {
    const queueTypeEl = document.getElementById("cakeQueueType");
    if (!queueTypeEl) {
        return;
    }
    const down = (kindDown ?? "").toString().trim().toLowerCase();
    const up = (kindUp ?? "").toString().trim().toLowerCase();
    queueTypeEl.textContent = formatDirectionalQueueTypeDisplay(down, up);
}

function requestCircuitById(onSuccess, onError) {
    listenOnce("CircuitByIdResult", (msg) => {
        if (!msg || !msg.ok) {
            if (onError) onError();
            return;
        }
        if (msg.id && msg.id !== circuit_id) {
            if (onError) onError();
            return;
        }
        onSuccess(msg.devices || []);
    });
    wsClient.send({ CircuitById: { id: circuit_id } });
}

function initExcludeRttToggle() {
    excludeRttToggle = document.getElementById("excludeRttToggle");
    if (!excludeRttToggle) return;

    const listenOnceMatch = (eventName, predicate, handler) => {
        const wrapped = (msg) => {
            if (!predicate(msg)) return;
            wsClient.off(eventName, wrapped);
            handler(msg);
        };
        wsClient.on(eventName, wrapped);
    };

    excludeRttToggle.addEventListener("change", () => {
        if (excludeRttBusy) return;
        const desired = !!excludeRttToggle.checked;
        excludeRttBusy = true;
        listenOnceMatch(
            "SetCircuitRttExcludedResult",
            (msg) => !msg?.circuit_id || msg.circuit_id === circuit_id,
            (msg) => {
                excludeRttBusy = false;
                if (!msg || !msg.ok) {
                    alert((msg && msg.message) ? msg.message : "Failed to update RTT exclusion");
                    excludeRttToggle.checked = !!excludeRttLastValue;
                    return;
                }
                excludeRttLastValue = desired;
            },
        );
        wsClient.send({ SetCircuitRttExcluded: { circuit_id, excluded: desired } });
    });
}

function connectPrivateChannel() {
    channelLink = new DirectChannel({
        CircuitWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        latestCircuitQooScore = toNumber(msg.qoo_score, NaN);
        if (!Number.isFinite(latestCircuitQooScore)) {
            latestCircuitQooScore = null;
        }
        if (msg.devices !== null) {
            latestCircuitDevices = msg.devices || [];
            fillLiveDevices(msg.devices);
            updateSpeedometer(msg.devices);
            pushQueuingActivitySample();
            if (excludeRttToggle && msg.rtt_excluded !== undefined) {
                excludeRttLastValue = !!msg.rtt_excluded;
                excludeRttToggle.checked = excludeRttLastValue;
            }
        }
        if (qooGauge !== null) {
            qooGauge.update(msg.qoo_score);
        }
        updateQueuingActivityCards();
    });
}

function fullIpList(circuits) {
    let ipList = [];
    circuits.forEach((circuit) => {
        circuit.ipv4.forEach((ip) => {
            ipList.push([ipToString(ip[0]), circuit.device_id]);
        });
        circuit.ipv6.forEach((ip) => {
            ipList.push([ipToString(ip[0]), circuit.device_id]);
        });
    });
    return ipList;
}

function connectPingers(circuits) {
    let ipList = fullIpList(circuits);

    pinger = new DirectChannel({
        PingMonitor: {
            ips: ipList
        }
    },(msg) => {
        //console.log(msg);
        if (msg.ip != null && msg.ip !== "test") {
            // Stats Updates
            if (devicePings[msg.ip] === undefined) {
                devicePings[msg.ip] = {
                    count: 0,
                    timeout: 0,
                    success: 0,
                    times: [],
                }
            }

                devicePings[msg.ip].count++;
                if (msg.result === "NoResponse") {
                    devicePings[msg.ip].timeout++;
                } else {
                    devicePings[msg.ip].success++;
                    const pingNanos = toNumber(msg.result.Ping.time_nanos, 0);
                    devicePings[msg.ip].times.push(pingNanos);
                    if (devicePings[msg.ip].times.length > 300) {
                        devicePings[msg.ip].times.shift();
                    }
                    let graphId = "pingGraph_" + msg.result.Ping.label;
                    let graph = deviceGraphs[graphId];
                    if (graph !== undefined) {
                        graph.update(pingNanos);
                    }
                }

            // Visual Updates
            let target = document.getElementById("ip_" + msg.ip);
            if (target != null) {
                let myPing = devicePings[msg.ip];
                if (myPing.count === myPing.timeout) {
                    target.innerHTML = "<i class='fa fa-minus-circle text-secondary' data-bs-toggle='tooltip' data-bs-placement='top' title='No ping response - this is normal for many ISPs'></i>";
                } else {
                    let loss = ((myPing.timeout / myPing.count) * 100);
                    let lossStr = loss.toFixed(1);
                    let avg = 0;
                    myPing.times.forEach((time) => {
                        avg += time;
                    });
                    avg = avg / myPing.times.length;
                    let lossColor = "text-success";
                    if (loss > 0 && loss < 10) {
                        lossColor = "text-warning";
                    } else if (loss >= 10) {
                        lossColor = "text-danger";
                    }
                    let pingRamp = Math.min(avg / 200, 1);
                    let pingColor = lerpGreenToRedViaOrange(pingRamp, 1);
                    target.innerHTML = "<i class='fa fa-check text-success' data-bs-toggle='tooltip' data-bs-placement='top' title='Device is responding to pings'></i> <span class='tiny'><span class='" + lossColor + "'>" + lossStr + "%</span> / <span style='color: " + pingColor + "'>" + scaleNanos(avg) + "</span></span>";
                }
            }
        }
    });
}

function connectFlowChannel() {
    flowChannel = new DirectChannel({
        FlowsByCircuit: {
            circuit: circuit_id
        }
    }, (msg) => {
        latestFlowMsg = msg;
        ingestTrafficRows(msg);
        if (isSankeyTabActive()) {
            ensureFlowSankey();
            applyFlowSankeyMessage(msg);
        } else {
            $("#activeFlowCount").text(getRenderableSankeyFlowCount(msg));
        }
        if (isTrafficTabActive()) {
            renderTrafficTab();
        } else {
            updateTrafficCountBadge();
            updateTrafficPaginationControls();
        }
        updateQueuingActivityCards();
    });
}

function initFlowFilters() {
    const hideSmallFlows = document.getElementById("hideSmallFlows");
    const pageSize = document.getElementById("trafficPageSize");
    const prev = document.getElementById("trafficPagePrev");
    const next = document.getElementById("trafficPageNext");
    const trafficContainer = document.getElementById("allTraffic");

    if (hideSmallFlows) {
        hideSmallFlows.addEventListener("change", () => {
            trafficCurrentPage = 1;
            renderTrafficTab();
        });
    }
    if (pageSize) {
        pageSize.value = String(trafficPageSize);
        pageSize.addEventListener("change", () => {
            const parsed = parseInt(pageSize.value, 10);
            trafficPageSize = Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_TRAFFIC_PAGE_SIZE;
            trafficCurrentPage = 1;
            renderTrafficTab();
        });
    }
    if (prev) {
        prev.addEventListener("click", () => {
            if (trafficCurrentPage > 1) {
                trafficCurrentPage--;
                renderTrafficTab();
            }
        });
    }
    if (next) {
        next.addEventListener("click", () => {
            const totalPages = getTrafficTotalPages();
            if (trafficCurrentPage < totalPages) {
                trafficCurrentPage++;
                renderTrafficTab();
            }
        });
    }
    if (trafficContainer) {
        trafficContainer.addEventListener("click", (event) => {
            const button = event.target.closest(".flow-rtt-exclude-btn");
            if (!button) {
                return;
            }
            event.preventDefault();
            event.stopPropagation();
            const remoteIp = String(button.dataset.remoteIp || "").trim();
            if (!remoteIp) {
                return;
            }
            openFlowRttExcludeWizard({ remoteIp, sourceLabel: "Circuit" });
        });
    }
}

let movingAverages = new Map();
let prevFlowBytes = new Map();
let tickCount = 0;
let trafficSortColumn = 'rate'; // Default sort by rate
let trafficSortDirection = 'desc'; // 'asc' or 'desc'
let latestTrafficRows = [];
let trafficCurrentPage = 1;
let trafficPageSize = DEFAULT_TRAFFIC_PAGE_SIZE;

function diffToNumber(current, previous, fallback = 0) {
    if (typeof current === "bigint" && typeof previous === "bigint") {
        return toNumber(current - previous, fallback);
    }
    return toNumber(current, fallback) - toNumber(previous, fallback);
}

function formatQooScore(score0to100, fallback = "-") {
    if (score0to100 === null || score0to100 === undefined) {
        return fallback;
    }
    const numeric = Number(score0to100);
    // QoqScores uses 255 for unknown.
    if (!Number.isFinite(numeric) || numeric === 255) {
        return fallback;
    }
    const clamped = Math.min(100, Math.max(0, Math.round(numeric)));
    const color = colorByQoqScore(clamped);
    return "<span class='muted' style='color: " + color + "'>■</span>" + clamped;
}

function formatRttNanos(rttNanos) {
    const n = toNumber(rttNanos, 0);
    if (n === 0) {
        return "<span class='muted' style='color: var(--bs-border-color)'>■</span>-";
    }
    const rttInMs = n / 1000000;
    const color = colorByRttMs(rttInMs);
    return "<span class='muted' style='color: " + color + "'>■</span>" + scaleNanos(n);
}

function formatRttPair(p50Nanos, p95Nanos) {
    const p50 = toNumber(p50Nanos, 0);
    const p95 = toNumber(p95Nanos, 0);
    if (p50 === 0 && p95 === 0) {
        return "-";
    }
    return formatRttNanos(p50) + " / " + scaleNanos(p95);
}

function visibleTrafficRows() {
    if (!hideSmallFlowsEnabled()) {
        return latestTrafficRows;
    }
    return latestTrafficRows.filter((row) => row.downBps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS || row.upBps > TRAFFIC_FLOW_HIDE_THRESHOLD_BPS);
}

function hideSmallFlowsEnabled() {
    return document.getElementById("hideSmallFlows")?.checked ?? false;
}

function sortTrafficRows(rows) {
    rows.sort((a, b) => {
        let aVal = a.sortKeys[trafficSortColumn];
        let bVal = b.sortKeys[trafficSortColumn];
        if (typeof aVal === "string" && typeof bVal === "string") {
            aVal = aVal.toLowerCase();
            bVal = bVal.toLowerCase();
        }
        if (trafficSortDirection === "asc") {
            return aVal < bVal ? -1 : aVal > bVal ? 1 : 0;
        }
        return aVal > bVal ? -1 : aVal < bVal ? 1 : 0;
    });
}

function getTrafficTotalPages() {
    return Math.max(1, Math.ceil(visibleTrafficRows().length / trafficPageSize));
}

function updateTrafficPaginationControls() {
    const totalPages = getTrafficTotalPages();
    trafficCurrentPage = Math.min(Math.max(1, trafficCurrentPage), totalPages);

    const info = document.getElementById("trafficPageInfo");
    const prev = document.getElementById("trafficPagePrev");
    const next = document.getElementById("trafficPageNext");
    if (info) {
        info.textContent = `Page ${trafficCurrentPage} / ${totalPages}`;
    }
    if (prev) {
        prev.disabled = trafficCurrentPage <= 1;
    }
    if (next) {
        next.disabled = trafficCurrentPage >= totalPages;
    }
}

function updateTrafficCountBadge() {
    $("#trafficFlowCount").text(visibleTrafficRows().length);
}

function ingestTrafficRows(msg) {
    tickCount++;
    const rows = [];
    const seenFlowKeys = new Set();
    const flowList = Array.isArray(msg?.flows) ? msg.flows : [];

    flowList.forEach((flow) => {
        const flowKey = `${flow?.[0]?.protocol_name || ""}${flow?.[0]?.row_id || ""}`;
        seenFlowKeys.add(flowKey);

        let down = toNumber(flow?.[1]?.rate_estimate_bps?.down, 0);
        let up = toNumber(flow?.[1]?.rate_estimate_bps?.up, 0);
        const prev = prevFlowBytes.get(flowKey);
        if (prev) {
            const ticks = tickCount - prev.tick;
            if (ticks === 1) {
                down = diffToNumber(flow[1].bytes_sent.down, prev.downBytes, 0) * 8;
                up = diffToNumber(flow[1].bytes_sent.up, prev.upBytes, 0) * 8;
            } else if (ticks > 1) {
                down = diffToNumber(flow[1].bytes_sent.down, prev.downBytes, 0) * 8 / ticks;
                up = diffToNumber(flow[1].bytes_sent.up, prev.upBytes, 0) * 8 / ticks;
            }
        }
        if (down < 0) down = 0;
        if (up < 0) up = 0;

        prevFlowBytes.set(flowKey, {
            downBytes: flow?.[1]?.bytes_sent?.down,
            upBytes: flow?.[1]?.bytes_sent?.up,
            tick: tickCount,
        });

        const currentRate = down + up;
        const average = movingAverages.get(flowKey) || { values: [], total: 0 };
        average.values.push(currentRate);
        average.total += currentRate;
        if (average.values.length > 10) {
            average.total -= average.values.shift();
        }
        movingAverages.set(flowKey, average);

        const lastSeenNanos = toNumber(flow?.[0]?.last_seen_nanos, 0);
        if (lastSeenNanos > RECENT_TRAFFIC_FLOW_WINDOW_NANOS) {
            return;
        }

        const tcpRetransmitsDown = toNumber(flow?.[1]?.tcp_retransmits?.down, 0);
        const tcpRetransmitsUp = toNumber(flow?.[1]?.tcp_retransmits?.up, 0);
        const packetsSentDown = toNumber(flow?.[1]?.packets_sent?.down, 0);
        const packetsSentUp = toNumber(flow?.[1]?.packets_sent?.up, 0);
        const retransmitDownPct = tcpRetransmitsDown > 0 && packetsSentDown > 0 ? tcpRetransmitsDown / packetsSentDown : 0;
        const retransmitUpPct = tcpRetransmitsUp > 0 && packetsSentUp > 0 ? tcpRetransmitsUp / packetsSentUp : 0;
        const bytesSentDown = toNumber(flow?.[1]?.bytes_sent?.down, 0);
        const bytesSentUp = toNumber(flow?.[1]?.bytes_sent?.up, 0);
        const rttDownNanos = toNumber(flow?.[1]?.rtt?.[0]?.nanoseconds, 0);
        const rttUpNanos = toNumber(flow?.[1]?.rtt?.[1]?.nanoseconds, 0);
        const qoq = flow?.[1]?.qoq || null;
        const qooDown = qoq ? qoq.download_total : null;
        const qooUp = qoq ? qoq.upload_total : null;
        const remoteIp = String(flow?.[0]?.remote_ip || "").trim();

        rows.push({
            protocolName: flow?.[0]?.protocol_name || "",
            downBps: down,
            upBps: up,
            bytesSentDown,
            bytesSentUp,
            packetsSentDown,
            packetsSentUp,
            retransmitDownPct,
            retransmitUpPct,
            rttDownNanos,
            rttUpNanos,
            qooDown,
            qooUp,
            asnName: flow?.[0]?.asn_name || "",
            asnCountry: flow?.[0]?.asn_country || "",
            remoteIp,
            opacity: 1.0 - Math.min(1, lastSeenNanos / RECENT_TRAFFIC_FLOW_WINDOW_NANOS),
            sortKeys: {
                protocol: flow?.[0]?.protocol_name || "",
                rate: average.values.length > 0 ? average.total / average.values.length : currentRate,
                bytes: bytesSentDown + bytesSentUp,
                packets: packetsSentDown + packetsSentUp,
                retransmits: retransmitDownPct + retransmitUpPct,
                rtt: rttDownNanos + rttUpNanos,
                qoo: (typeof qooDown === "number" ? qooDown : 0) + (typeof qooUp === "number" ? qooUp : 0),
                asn: flow?.[0]?.asn_name || "",
                country: flow?.[0]?.asn_country || "",
                ip: remoteIp,
            },
        });
    });

    Array.from(prevFlowBytes.keys()).forEach((key) => {
        if (!seenFlowKeys.has(key)) {
            prevFlowBytes.delete(key);
            movingAverages.delete(key);
        }
    });

    latestTrafficRows = rows;
}

function renderTrafficTab() {
    const target = document.getElementById("allTraffic");
    if (!target) {
        return;
    }

    const visibleRows = visibleTrafficRows().slice();
    sortTrafficRows(visibleRows);
    const totalPages = Math.max(1, Math.ceil(visibleRows.length / trafficPageSize));
    trafficCurrentPage = Math.min(Math.max(1, trafficCurrentPage), totalPages);
    const startIndex = (trafficCurrentPage - 1) * trafficPageSize;
    const pagedRows = visibleRows.slice(startIndex, startIndex + trafficPageSize);

    let tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");

    let table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight");
    let thead = document.createElement("thead", "small");
    thead.style.fontSize = "0.8em";

    const createSortableHeader = (text, sortKey, colspan = 1) => {
        let th = theading(text, colspan);
        th.style.cursor = "pointer";
        th.onclick = () => {
            if (trafficSortColumn === sortKey) {
                trafficSortDirection = trafficSortDirection === "asc" ? "desc" : "asc";
            } else {
                trafficSortColumn = sortKey;
                trafficSortDirection = "desc";
            }
            trafficCurrentPage = 1;
            renderTrafficTab();
        };
        if (trafficSortColumn === sortKey) {
            th.innerHTML += trafficSortDirection === "asc" ? " ▲" : " ▼";
        }
        return th;
    };

    thead.appendChild(createSortableHeader("Protocol", "protocol"));
    thead.appendChild(createSortableHeader("Current Rate (d/u)", "rate", 2));
    thead.appendChild(createSortableHeader("Total Bytes (d/u)", "bytes", 2));
    thead.appendChild(createSortableHeader("Total Packets (d/u)", "packets", 2));
    thead.appendChild(createSortableHeader("TCP rxmit (d/u)", "retransmits", 2));
    thead.appendChild(createSortableHeader("RTT (d/u)", "rtt", 2));
    thead.appendChild(createSortableHeader("QoO (d/u)", "qoo", 2));
    thead.appendChild(createSortableHeader("ASN", "asn"));
    thead.appendChild(createSortableHeader("Country", "country"));
    thead.appendChild(createSortableHeader("Remote IP", "ip"));
    thead.appendChild(theading("RTT Exclude"));
    table.appendChild(thead);

    let tbody = document.createElement("tbody");
    if (pagedRows.length === 0) {
        const empty = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 17;
        td.classList.add("text-center", "text-muted", "small");
        td.textContent = "No recent flows match the current filter.";
        empty.appendChild(td);
        tbody.appendChild(empty);
    } else {
        pagedRows.forEach((rowData) => {
            let row = document.createElement("tr");
            row.classList.add("small");
            row.style.opacity = rowData.opacity;

            row.appendChild(simpleRow(rowData.protocolName));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.downBps, plan.down)));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.upBps, plan.up)));
            row.appendChild(simpleRow(scaleNumber(rowData.bytesSentDown)));
            row.appendChild(simpleRow(scaleNumber(rowData.bytesSentUp)));
            row.appendChild(simpleRow(scaleNumber(rowData.packetsSentDown)));
            row.appendChild(simpleRow(scaleNumber(rowData.packetsSentUp)));
            row.appendChild(simpleRowHtml(rowData.retransmitDownPct > 0 ? formatRetransmit(rowData.retransmitDownPct) : "-"));
            row.appendChild(simpleRowHtml(rowData.retransmitUpPct > 0 ? formatRetransmit(rowData.retransmitUpPct) : "-"));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rttDownNanos)));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rttUpNanos)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qooDown)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qooUp)));
            row.appendChild(simpleRow(rowData.asnName));
            row.appendChild(simpleRow(rowData.asnCountry));
            row.appendChild(simpleRow(rowData.remoteIp));

            const td = document.createElement("td");
            td.classList.add("text-center");
            const button = document.createElement("button");
            button.type = "button";
            button.className = "btn btn-outline-secondary btn-sm flow-rtt-exclude-btn";
            button.textContent = "Exclude";
            button.disabled = !rowData.remoteIp;
            button.title = "Open a wizard to exclude RTT samples for this remote IP/CIDR (requires saving in Flow Tracking config).";
            button.dataset.remoteIp = rowData.remoteIp;
            td.appendChild(button);
            row.appendChild(td);

            tbody.appendChild(row);
        });
    }

    table.appendChild(tbody);
    tableWrap.appendChild(table);
    clearDiv(target);
    target.appendChild(tableWrap);
    updateTrafficCountBadge();
    updateTrafficPaginationControls();
}

function updateSpeedometer(devices) {
    let totalDown = 0;
    let totalUp = 0;
    let planDown = 0;
    let planUp = 0;
    let retransmitsDown = 0;
    let retransmitsUp = 0;
    devices.forEach((device) => {
        const deviceDown = toNumber(device.bytes_per_second.down, 0);
        const deviceUp = toNumber(device.bytes_per_second.up, 0);
        totalDown += deviceDown;
        totalUp += deviceUp;
        planDown = Math.max(planDown, toNumber(device.plan.down, 0));
        planUp = Math.max(planUp, toNumber(device.plan.up, 0));
        retransmitsDown += toNumber(device.tcp_retransmits.down, 0);
        retransmitsUp += toNumber(device.tcp_retransmits.up, 0);

        let throughputGraph = deviceGraphs["throughputGraph_" + device.device_id];
        if (throughputGraph !== undefined) {
            throughputGraph.update(deviceDown * 8, deviceUp * 8);
        }

        let retransmitGraph = deviceGraphs["tcpRetransmitsGraph_" + device.device_id];
        if (retransmitGraph !== undefined) {
            retransmitGraph.update(
                toNumber(device.tcp_retransmits.down, 0),
                toNumber(device.tcp_retransmits.up, 0)
            );
        }
    });
    speedometer.update(totalDown * 8, totalUp * 8, planDown, planUp);
    totalThroughput.update(totalDown * 8, totalUp * 8);
    totalRetransmits.update(retransmitsDown, retransmitsUp);
}

function fillLiveDevices(devices) {
    devices.forEach((device) => {
        let last_seen = document.getElementById("last_seen_" + device.device_id);
        let throughputDown = document.getElementById("throughputDown_" + device.device_id);
        let throughputUp = document.getElementById("throughputUp_" + device.device_id);
        let rttDown = document.getElementById("rttDown_" + device.device_id);
        let rttUp = document.getElementById("rttUp_" + device.device_id);
        let tcp_retransmitsDown = document.getElementById("tcp_retransmitsDown_" + device.device_id);
        let tcp_retransmitsUp = document.getElementById("tcp_retransmitsUp_" + device.device_id);

        if (last_seen !== null) {
            last_seen.innerHTML = formatLastSeen(device.last_seen_nanos);
        }

        if (throughputDown !== null) {
            throughputDown.innerHTML = formatThroughput(
                toNumber(device.bytes_per_second.down, 0) * 8,
                toNumber(device.plan.down, 0)
            );
        }

        if (throughputUp !== null) {
            throughputUp.innerHTML = formatThroughput(
                toNumber(device.bytes_per_second.up, 0) * 8,
                toNumber(device.plan.up, 0)
            );
        }

        if (rttDown !== null) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const curP95 = device.rtt_current_p95_nanos || {};
            const totP50 = device.rtt_total_p50_nanos || {};
            const totP95 = device.rtt_total_p95_nanos || {};
            rttDown.innerHTML = formatRttMetricBlock(
                formatRttPair(curP50.down, curP95.down),
                formatRttPair(totP50.down, totP95.down)
            );
        }

        if (rttUp !== null) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const curP95 = device.rtt_current_p95_nanos || {};
            const totP50 = device.rtt_total_p50_nanos || {};
            const totP95 = device.rtt_total_p95_nanos || {};
            rttUp.innerHTML = formatRttMetricBlock(
                formatRttPair(curP50.up, curP95.up),
                formatRttPair(totP50.up, totP95.up)
            );
        }

        if (tcp_retransmitsDown !== null) {
            tcp_retransmitsDown.innerHTML = formatRetransmit(device.tcp_retransmits.down);
        }

        if (tcp_retransmitsUp !== null) {
            tcp_retransmitsUp.innerHTML = formatRetransmit(device.tcp_retransmits.up);
        }

        // Local RTT histogram (5-minute window, p50 samples)
        let rttHistogram = deviceGraphs["rttHistogramGraph_" + device.device_id];
        if (rttHistogram !== undefined) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const downNanos = toNumber(curP50.down, 0);
            const upNanos = toNumber(curP50.up, 0);
            const samples = [];
            if (downNanos > 0) samples.push(downNanos / 1000000);
            if (upNanos > 0) samples.push(upNanos / 1000000);
            rttHistogram.updateManyMs(samples);
        }
    });
}

function formatRttMetricBlock(currentText, totalText) {
    return "<div class='lqos-rtt-metric'>" +
        "<div class='lqos-rtt-metric-line'>" +
        "<span class='lqos-rtt-metric-label'>C:</span>" +
        "<span class='lqos-rtt-metric-value'>" + currentText + "</span>" +
        "</div>" +
        "<div class='lqos-rtt-metric-line text-secondary'>" +
        "<span class='lqos-rtt-metric-label'>T:</span>" +
        "<span class='lqos-rtt-metric-value'>" + totalText + "</span>" +
        "</div>" +
        "</div>";
}

function initialDevices(circuits) {
    let target = document.getElementById("devices");
    clearDiv(target);
    deviceGraphs = {};
    deviceGraphSpecs = [];
    deviceGraphsInitialized = false;

    circuits.forEach((circuit) => {
        let outer = document.createElement("div");
        outer.classList.add("col-12", "mb-3");
        target.appendChild(outer);

        let row = document.createElement("div");
        row.classList.add("row", "g-2");
        outer.appendChild(row);

        let d = document.createElement("div");
        d.classList.add("col-3");
        row.appendChild(d);

        // Device Information Section

        let name = document.createElement("h5");
        name.classList.add("redactable");
        name.innerHTML = "<i class='fa fa-computer'></i> " + circuit.device_name;
        d.appendChild(name);

        let infoTableWrap = document.createElement("div");
        infoTableWrap.classList.add("lqos-table-wrap");

        let infoTable = document.createElement("table");
        infoTable.classList.add("lqos-table", "lqos-table-tight");
        let tbody = document.createElement("tbody");

        // MAC Row
        let tr = document.createElement("tr");
        let td = document.createElement("td");
        td.textContent = "MAC Address";
        td.classList.add("table-label-cell");
        tr.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.classList.add("redactable");
        td.colSpan = 2;
        td.innerHTML = circuit.mac;
        tr.appendChild(td);
        tbody.appendChild(tr);

        // Comment Row
        let tr2 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "Comment";
        td.classList.add("table-label-cell");
        tr2.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.colSpan = 2;
        td.innerHTML = circuit.comment;
        tr2.appendChild(td);
        tbody.appendChild(tr2);

        // IPv4 Row
        let tr3 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "IPv4 Address(es)";
        td.classList.add("table-label-cell");
        tr3.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.colSpan = 2;
        let ipv4Table = document.createElement("table");
        ipv4Table.classList.add("lqos-table", "lqos-table-tight");
        let ipv4Body = document.createElement("tbody");
        circuit.ipv4.forEach((ip) => {
            const ipStr = ipToString(ip[0]);
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.textContent = ipStr + "/" + ip[1];
            label.classList.add("redactable");
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ipStr;
            value.innerText = "-";
            tr.appendChild(value);
            ipv4Body.appendChild(tr);
        });
        if (circuit.ipv4.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv4 addresses assigned";
            tr.appendChild(label);
            ipv4Body.appendChild(tr);
        }
        ipv4Table.appendChild(ipv4Body);
        td.appendChild(ipv4Table);

        tr3.appendChild(td);
        tbody.appendChild(tr3);

        // IPv6 Row
        let tr4 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "IPv6 Address(es)";
        td.classList.add("table-label-cell");
        tr4.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.colSpan = 2;

        let ipv6 = document.createElement("table");
        ipv6.classList.add("lqos-table", "lqos-table-tight");
        let ipv6Body = document.createElement("tbody");
        circuit.ipv6.forEach((ip) => {
            const ipStr = ipToString(ip[0]);
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.textContent = ipStr + "/" + ip[1];
            label.classList.add("redactable");
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ipStr;
            value.innerText = "-";
            tr.appendChild(value);
            ipv6Body.appendChild(tr);
        });
        if (circuit.ipv6.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv6 addresses assigned";
            tr.appendChild(label);
            ipv6Body.appendChild(tr);
        }
        ipv6.appendChild(ipv6Body);
        td.appendChild(ipv6);

        /*let ipv6 = "";
        circuit.ipv6.forEach((ip) => {
            ipv6 += ip[0] + "/" + ip[1] + "<br>";
        });
        if (ipv6 === "") ipv6 = "No IPv6 addresses assigned";
        td.innerHTML = ipv6;*/
        tr4.appendChild(td);
        tbody.appendChild(tr4);

        // Placeholder for Last Seen
        let tr8 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "Last Seen";
        td.classList.add("table-label-cell");
        tr8.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.colSpan = 2;
        td.id = "last_seen_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr8.appendChild(td);
        tbody.appendChild(tr8);

        // Placeholder for throughput
        let tr5 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "Throughput";
        td.classList.add("table-label-cell");
        tr5.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.id = "throughputDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.id = "throughputUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        tbody.appendChild(tr5);

        // Placeholder for RTT
        let tr6 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "RTT P50/P95";
        td.classList.add("table-label-cell");
        tr6.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell", "lqos-rtt-metric-cell");
        td.id = "rttDown_" + circuit.device_id;
        td.innerHTML = formatRttMetricBlock("Sampling...", "Sampling...");
        tr6.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell", "lqos-rtt-metric-cell");
        td.id = "rttUp_" + circuit.device_id;
        td.innerHTML = formatRttMetricBlock("Sampling...", "Sampling...");
        tr6.appendChild(td);
        tbody.appendChild(tr6);

        // Placeholder for TCP Retransmits
        let tr7 = document.createElement("tr");
        td = document.createElement("td");
        td.textContent = "TCP Re-Xmits";
        td.classList.add("table-label-cell");
        tr7.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.id = "tcp_retransmitsDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        td = document.createElement("td");
        td.classList.add("table-value-cell");
        td.id = "tcp_retransmitsUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        tbody.appendChild(tr7);

        infoTable.appendChild(tbody);
        infoTableWrap.appendChild(infoTable);
        d.appendChild(infoTableWrap);

        // Graph container (2x2)
        let graphCol = document.createElement("div");
        graphCol.classList.add("col-9");
        row.appendChild(graphCol);

        let graphRow = document.createElement("div");
        graphRow.classList.add("row", "g-2");
        graphCol.appendChild(graphRow);

        function addGraph(divId, graphFactory) {
            let col = document.createElement("div");
            col.classList.add("col-6");
            let div = document.createElement("div");
            div.id = divId;
            div.style.height = "250px";
            div.innerHTML = loadingBlockHtml("Loading chart…", "lqos-loading-block-sm");
            col.appendChild(div);
            graphRow.appendChild(col);
            deviceGraphSpecs.push({
                id: divId,
                factory: graphFactory,
            });
        }

        addGraph("throughputGraph_" + circuit.device_id, (id) => new CircuitTotalGraph(id, "Throughput"));
        addGraph("tcpRetransmitsGraph_" + circuit.device_id, (id) => new CircuitRetransmitGraph(id, "Retransmits"));
        addGraph("rttHistogramGraph_" + circuit.device_id, (id) => new WindowedLatencyHistogram(id, "RTT Histogram", 300000));
        addGraph("pingGraph_" + circuit.device_id, (id) => new DevicePingHistogram(id));

    });
}

function initialFunnel(parentNode) {
    funnelParentNodeName = parentNode;
    listenOnce("NetworkTreeLite", (msg) => {
        renderFunnel(resolveFunnelState(msg, parentNode));
        if (funnelSubscription) {
            funnelSubscription.dispose();
        }
        funnelSubscription = subscribeWS(["NetworkTreeLite"], onTreeEvent);
    });
    wsClient.send({ NetworkTreeLite: {} });
}

function onTreeEvent(msg) {
    if (msg.event !== "NetworkTreeLite" || !funnelParentNodeName) {
        return;
    }

    const state = resolveFunnelState(msg, funnelParentNodeName);
    const nextParents = state ? state.parentIndexes : [];
    const nextSignature = state ? state.parentSignature : [];
    const shouldRebuild =
        !arrayEquals(nextParents, funnelParents) ||
        !arrayEquals(nextSignature, funnelParentSignature);

    if (shouldRebuild) {
        renderFunnel(state);
    }

    funnelParents.forEach((parent) => {
        const nodeEntry = msg.data[parent];
        if (!nodeEntry || !nodeEntry[1] || !funnelGraphs[parent]) return;
        let myMessage = nodeEntry[1];
        let tpGraph = funnelGraphs[parent].tp;
        let rxmitGraph = funnelGraphs[parent].rxmit;
        let rttGraph = funnelGraphs[parent].rtt;

        tpGraph.update(
            toNumber(myMessage.current_throughput[0], 0) * 8,
            toNumber(myMessage.current_throughput[1], 0) * 8
        );
        let rxmit = [0, 0];
        const packetsDown = toNumber(myMessage.current_tcp_packets[0], 0);
        const packetsUp = toNumber(myMessage.current_tcp_packets[1], 0);
        const retransmitsDown = toNumber(myMessage.current_retransmits[0], 0);
        const retransmitsUp = toNumber(myMessage.current_retransmits[1], 0);
        if (retransmitsDown > 0 && packetsDown > 0) {
            rxmit[0] = (retransmitsDown / packetsDown) * 100.0;
        }
        if (retransmitsUp > 0 && packetsUp > 0) {
            rxmit[1] = (retransmitsUp / packetsUp) * 100.0;
        }
        rxmitGraph.update(rxmit[0], rxmit[1]);
        rttGraph.updateManyMs(myMessage.rtts);
    });
}

function subscribeToCake() {
    let noDataTimeout = null;
    let hasReceivedData = false;
    
    // Function to show "Queue not loaded" message
    function showNoQueueMessage() {
        const cakeTab = document.getElementById("cake");
        if (cakeTab && !hasReceivedData) {
            cakeTab.innerHTML = '<div class="row"><div class="col-12 text-center mt-5"><h4>Queue not loaded.</h4><p class="text-muted">The shaper queue for this circuit has not been created yet.</p></div></div>';
        }
    }
    
    // Set a timeout to show the message if no data arrives within 3 seconds
    noDataTimeout = setTimeout(showNoQueueMessage, 3000);
    
    cakeChannel = new DirectChannel({
        CakeWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        //console.log(msg);
        latestCakeMsg = msg;
        setQueueTypeDisplayFromKinds(msg?.kind_down, msg?.kind_up);
        
        // Clear the timeout and set flag that we've received data
        if (noDataTimeout) {
            clearTimeout(noDataTimeout);
            noDataTimeout = null;
        }
        
        // If this is the first data received, restore the original HTML structure
        if (!hasReceivedData) {
            hasReceivedData = true;
            if (!updateCakeTabAvailability(msg)) {
                return;
            }
        }

        ensureCakeGraphs();
        applyCakeMessage(msg);
    });
}

function wireupAnalysis(circuits) {
    let ipAddresses = fullIpList(circuits);
    let list = document.createElement("div");
    let listBtn = document.createElement("button");
    listBtn.type = "button";
    listBtn.id = "CaptureTopBtn";
    listBtn.classList.add("btn", "btn-secondary", "dropdown-toggle", "btn-sm");
    listBtn.setAttribute("data-bs-toggle", "dropdown");
    listBtn.innerHTML = "<i class='fa fa-search'></i> Packet Capture";
    list.appendChild(listBtn);

    let listUl = document.createElement("ul");
    listUl.classList.add("dropdown-menu", "dropdown-menu-sized");
    ipAddresses.forEach((ip) => {
        let entry = document.createElement("li");
        let item = document.createElement("a");
        item.classList.add("dropdown-item");
        item.innerHTML = "<i class='fa fa-search'></i> Capture packets from <span class='redactable'>" + ip[0] + "</span>";
        let address = ip[0]; // For closure capture
        item.onclick = () => {
            //console.log("Clicky " + address);
            listenOnce("RequestAnalysisResult", (msg) => {
                const data = msg ? msg.data : null;
                const okData = data && data.Ok ? data.Ok : null;
                if (!okData) {
                    alert("Packet capture is already active for another IP. Please try again when it is finished.")
                    return;
                }
                let counter = parseInt(okData.countdown) + 1;
                let sessionId = okData.session_id;
                let btn = document.getElementById("CaptureTopBtn");
                btn.disabled = true;
                btn.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Capturing Packets (" + counter + ")";
                let interval = setInterval(() => {
                    counter--;
                    if (counter === -1) {
                        clearInterval(interval);
                        btn.disabled = false;
                        btn.innerHTML = "<i class='fa fa-download'></i> Download Packet Capture for <span class='redactable'>" + address + "</span>";
                        btn.classList.remove("btn-secondary");
                        btn.classList.add("btn-success");
                        btn.onclick = () => {
                            let url = "/local-api/pcapDump/" + sessionId;
                            download(url, "capture.pcap");
                            //console.log(url);

                            // Restore the buttons
                            requestCircuitById((circuits) => {
                                wireupAnalysis(circuits);
                            });
                        }
                        return;
                    }
                    btn.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Capturing Packets (" + counter + ")";
                }, 1000);
            });
            wsClient.send({ RequestAnalysis: { ip: address } });
        }
        entry.appendChild(item);
        listUl.appendChild(entry);
    });
    list.appendChild(listUl);
    let parent = document.getElementById("captureButton");
    clearDiv(parent);
    parent.appendChild(list);
}

function download(dataurl, filename) {
    const link = document.createElement("a");
    link.href = dataurl;
    link.download = filename;
    link.click();
}

function loadInitial() {
    initTooltipsWithin(document);
    initExcludeRttToggle();
    initFlowFilters();
    initQueuingActivityControls();
    loadRttThresholds();
    requestCircuitById((circuits) => {
        let circuit = circuits[0];
        $("#circuitName").text(circuit.circuit_name);
        $("#circuitName").attr("title", circuit.circuit_name || "");
        applyParentNodeLink(circuit.parent_node);
        $("#bwMax").text(formatMbps(circuit.download_max_mbps) + " / " + formatMbps(circuit.upload_max_mbps));
        $("#bwMin").text(formatMbps(circuit.download_min_mbps) + " / " + formatMbps(circuit.upload_min_mbps));
        plan = {
            down: toNumber(circuit.download_max_mbps, 0),
            up: toNumber(circuit.upload_max_mbps, 0),
        };
        latestCircuitDevices = circuits;
        circuitSqmOverride = circuit.sqm_override || "";
        setQueueTypeDisplayFromKinds(latestCakeMsg?.kind_down, latestCakeMsg?.kind_up);
        initialDevices(circuits);
        speedometer = new BitsPerSecondGauge("bitsGauge", "Plan");
        qooGauge = new QooScoreGauge("qooGauge");
        totalThroughput = new CircuitTotalGraph("throughputGraph", "Total Circuit Throughput");
        totalRetransmits = new CircuitRetransmitGraph("rxmitGraph", "Total Circuit Retransmits");
        initTabLifecycle(circuit.parent_node);
        updateQueuingActivityCards();

        connectPrivateChannel();
        connectPingers(circuits);
        connectFlowChannel();
        subscribeToCake();
        wireupAnalysis(circuits);
    }, () => {
        alert("Circuit with id " + circuit_id + " not found");
    });
}

function cleanupCircuitPage() {
    if (channelLink) {
        channelLink.close();
        channelLink = null;
    }
    if (cakeChannel) {
        cakeChannel.close();
        cakeChannel = null;
    }
    if (pinger) {
        pinger.close();
        pinger = null;
    }
    if (flowChannel) {
        flowChannel.close();
        flowChannel = null;
    }
    if (funnelSubscription) {
        funnelSubscription.dispose();
        funnelSubscription = null;
    }
    if (queuingActivityGraph) {
        queuingActivityGraph.dispose();
        queuingActivityGraph = null;
    }
    Object.values(deviceGraphs).forEach((graph) => {
        if (graph && typeof graph.dispose === "function") {
            graph.dispose();
        }
    });
    deviceGraphs = {};
    deviceGraphSpecs = [];
    deviceGraphsInitialized = false;
}

window.addEventListener("beforeunload", cleanupCircuitPage);
loadInitial();
