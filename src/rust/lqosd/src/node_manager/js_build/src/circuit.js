// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";
import {clearDiv, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {
    formatRetransmitFraction,
    formatRtt,
    formatThroughput,
    lerpGreenToRedViaOrange,
    formatMbps,
    retransmitFractionFromSample,
} from "./helpers/scaling";
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
let funnelParentNodeId = null;
let excludeRttToggle = null;
let excludeRttLastValue = false;
let excludeRttBusy = false;
let latestCakeMsg = null;
let cakeGraphs = null;
let cakeQueueUnavailable = false;
let circuitSqmOverride = "";
let queuingActivityGraph = null;
let latestCircuitDevices = [];
let circuitConfigDevices = [];
let latestCircuitSummary = null;
let latestCircuitQooScore = null;
let latestSankeyFlowMsg = { flows: [] };
let latestTopAsnData = { total_asns: 0, rows: [] };
let latestTrafficPage = null;
let queuingActivityDirection = "down";
let deviceGraphSpecs = [];
let deviceGraphsInitialized = false;
let devicePollTimer = null;
let deviceRequestInFlight = false;
let sankeyPollTimer = null;
let sankeyRequestInFlight = false;
let topAsnPollTimer = null;
let topAsnRequestInFlight = false;
let trafficPollTimer = null;
let trafficRequestInFlight = false;
const DEFAULT_RTT_THRESHOLDS = { green_ms: 0, yellow_ms: 100, red_ms: 200 };
let currentRttThresholds = { ...DEFAULT_RTT_THRESHOLDS };
const wsClient = get_ws_client();
const RECENT_TRAFFIC_FLOW_WINDOW_NANOS = 30_000_000_000;
const TRAFFIC_FLOW_HIDE_THRESHOLD_BPS = 1024 * 1024;
const DEFAULT_TRAFFIC_PAGE_SIZE = 100;

function formatEthernetPortLabel(mbps) {
    const value = toNumber(mbps, 0);
    if (value >= 1000 && value % 1000 === 0) {
        return `${value / 1000}G`;
    }
    if (value >= 1000) {
        return `${(value / 1000).toFixed(1)}G`;
    }
    return `${Math.round(value)}M`;
}

function ethernetCapsPageHref(advisory) {
    const portLabel = encodeURIComponent(formatEthernetPortLabel(advisory?.negotiated_ethernet_mbps));
    return `/ethernet_caps.html?tier=${portLabel}`;
}

function ethernetTooltipHtml(advisory) {
    return (
        `Requested plan ${formatMbps(advisory.requested_download_mbps)} / ${formatMbps(advisory.requested_upload_mbps)} exceeded detected Ethernet speed. ` +
        `Shaping auto-capped to ${formatMbps(advisory.applied_download_mbps)} / ${formatMbps(advisory.applied_upload_mbps)}.`
    );
}

function formatPlanSpeedPair(downloadMbps, uploadMbps) {
    const down = toNumber(downloadMbps, 0).toFixed(1);
    const up = toNumber(uploadMbps, 0).toFixed(1);
    return `${down} / ${up} Mbps`;
}

function renderEthernetAdvisory(advisory) {
    const badge = document.getElementById("ethernetAdvisoryBadge");
    const badgeText = document.getElementById("ethernetAdvisoryBadgeText");
    if (!badge || !badgeText) {
        return;
    }

    const disposeTooltip = () => {
        if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) {
            return;
        }
        const existing = bootstrap.Tooltip.getInstance?.(badge);
        existing?.dispose();
    };

    if (!advisory?.auto_capped) {
        badge.classList.add("d-none");
        badgeText.textContent = "";
        badge.removeAttribute("title");
        badge.removeAttribute("data-bs-original-title");
        disposeTooltip();
        return;
    }

    const portLabel = formatEthernetPortLabel(advisory.negotiated_ethernet_mbps);
    const note = ethernetTooltipHtml(advisory);

    badgeText.textContent = portLabel;
    badge.classList.remove("d-none");
    badge.href = ethernetCapsPageHref(advisory);
    badge.setAttribute("title", note);
    badge.setAttribute("data-bs-original-title", note);
    badge.setAttribute("aria-label", `Review ${portLabel} Ethernet-limited circuits`);
    disposeTooltip();
    initTooltipsWithin(badge.parentElement || document);
}

function retransmitPacketsForNode(node, direction) {
    return toNumber(
        node.current_tcp_retransmit_packets?.[direction] ?? node.current_tcp_packets?.[direction],
        0,
    );
}

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

function clearPollingTimer(timerId) {
    if (timerId !== null) {
        window.clearInterval(timerId);
    }
    return null;
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

function currentCircuitRttP50Ms(direction) {
    const directional = direction === "up" ? "up" : "down";
    const nanos = toNumber(latestCircuitSummary?.rtt_current_p50_nanos?.[directional], 0);
    if (!(nanos > 0)) {
        return null;
    }
    return nanos / 1_000_000.0;
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
    return toNumber(latestCircuitSummary?.active_flow_count, 0);
}

function currentDirectionValue(pair, direction, fallback = 0) {
    return toNumber(pair?.[direction], fallback);
}

function currentQueuingActivitySnapshot() {
    const throughputBps = currentDirectionValue(latestCircuitSummary?.bytes_per_second, queuingActivityDirection, 0) * 8;
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
}

function pushQueuingActivitySample() {
    if (!queuingActivityGraph || !latestCircuitSummary || !plan) {
        updateQueuingActivityCards();
        return;
    }

    queuingActivityGraph.pushSample({
        timestamp: Date.now(),
        throughputBps: {
            down: currentDirectionValue(latestCircuitSummary?.bytes_per_second, "down", 0) * 8,
            up: currentDirectionValue(latestCircuitSummary?.bytes_per_second, "up", 0) * 8,
        },
        actualThroughputBps: {
            down: currentDirectionValue(latestCircuitSummary?.actual_bytes_per_second, "down", 0) * 8,
            up: currentDirectionValue(latestCircuitSummary?.actual_bytes_per_second, "up", 0) * 8,
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

function queuingActivityDirectionColor(direction = queuingActivityDirection) {
    const normalized = direction === "up" ? "up" : "down";
    const fallback = normalized === "up" ? "#32d3bd" : "#4992ff";
    const paletteIndex = normalized === "up" ? 1 : 0;
    return window.graphPalette?.[paletteIndex] || fallback;
}

function updateQueuingActivityLegend() {
    const legendColor = queuingActivityDirectionColor();
    const enqueuedLegendEl = document.getElementById("queuingActivityLegendEnqueued");
    const throughputLegendEl = document.getElementById("queuingActivityLegendThroughput");
    if (enqueuedLegendEl) {
        enqueuedLegendEl.style.color = legendColor;
    }
    if (throughputLegendEl) {
        throughputLegendEl.style.color = legendColor;
    }
}

function applyQueuingDirection(direction) {
    queuingActivityDirection = direction === "up" ? "up" : "down";
    if (queuingActivityGraph) {
        queuingActivityGraph.setDirection(queuingActivityDirection);
    }
    updateQueuingActivityLegend();
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

function isDevicesTabActive() {
    return document.getElementById("devs-tab")?.classList.contains("active") ?? false;
}

function isTopAsnTabActive() {
    return document.getElementById("top-asns-tab")?.classList.contains("active") ?? false;
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
        applyFlowSankeyMessage(latestSankeyFlowMsg);
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
        applyDeviceLiveData(latestCircuitDevices);
    }
}

function ensureDeviceGraphs() {
    const target = document.getElementById("devs");
    if (!target || !isElementVisible(target)) {
        return;
    }
    runWhenRenderable(target, () => {
        if (!hasRenderableSize(target)) {
            return;
        }
        initializeDeviceGraphs();
        resizeDeviceGraphs();
    });
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

function normalizeNodeName(value) {
    return (value ?? "")
        .toString()
        .trim()
        .replace(/\s+/g, " ")
        .toLowerCase();
}

function resolveCircuitParentNode(payload, circuits) {
    const backendParent = payload?.parent_node;
    const backendName = backendParent?.name?.trim?.() || "";
    if (backendName) {
        return {
            id: backendParent?.id || null,
            name: backendName,
        };
    }

    if (!Array.isArray(circuits)) {
        return null;
    }

    const firstUsableParent = circuits.find((device) => normalizeNodeName(device?.parent_node).length > 0);
    const fallbackName = firstUsableParent?.parent_node?.trim() || "";
    if (!fallbackName) {
        return null;
    }

    return {
        id: null,
        name: fallbackName,
    };
}

function resolveFunnelState(msg, parentNode) {
    const data = msg && msg.data ? msg.data : [];
    const normalizedParentNodeId = normalizeNodeName(parentNode?.id);
    const normalizedParentNodeName = normalizeNodeName(parentNode?.name);
    if (!normalizedParentNodeId && !normalizedParentNodeName) {
        return null;
    }

    const namedEntry = data.find((node) => {
        const details = node[1];
        if (!details) {
            return false;
        }
        const nodeId = normalizeNodeName(details.id);
        if (normalizedParentNodeId && nodeId === normalizedParentNodeId) {
            return true;
        }
        if (normalizedParentNodeName && details.name === parentNode?.name) {
            return true;
        }
        return normalizedParentNodeName && normalizeNodeName(details.name) === normalizedParentNodeName;
    });
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
                    syncCircuitDetailSubscriptions();
                    return;
                }
                if (target === "#devs") {
                    ensureDeviceGraphs();
                    syncCircuitDetailSubscriptions();
                    return;
                }
                if (target === "#sankey") {
                    ensureFlowSankey();
                    applyFlowSankeyMessage(latestSankeyFlowMsg);
                    syncCircuitDetailSubscriptions();
                    return;
                }
                if (target === "#top-asns") {
                    renderTopAsnTab();
                    syncCircuitDetailSubscriptions();
                    return;
                }
                if (target === "#traffic") {
                    renderTrafficTab();
                    syncCircuitDetailSubscriptions();
                    return;
                }
                if (target === "#funnel") {
                    syncCircuitDetailSubscriptions();
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
                    syncCircuitDetailSubscriptions();
                }
            });
        });
    });

    window.requestAnimationFrame(() => {
        ensureQueuingActivityGraph();
        updateQueuingActivityCards();
        syncCircuitDetailSubscriptions();
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
        const payload = msg.data || {};
        onSuccess(payload);
    });
    wsClient.send({ CircuitById: { id: circuit_id } });
}

function applyCircuitSummary(summary) {
    latestCircuitSummary = summary || null;
    latestCircuitQooScore = toNumber(summary?.qoo_score, NaN);
    if (!Number.isFinite(latestCircuitQooScore)) {
        latestCircuitQooScore = null;
    }
    if (excludeRttToggle && summary?.rtt_excluded !== undefined) {
        excludeRttLastValue = !!summary.rtt_excluded;
        excludeRttToggle.checked = excludeRttLastValue;
    }
    if (speedometer) {
        speedometer.update(
            currentDirectionValue(summary?.actual_bytes_per_second, "down", 0) * 8,
            currentDirectionValue(summary?.actual_bytes_per_second, "up", 0) * 8,
            currentDirectionValue(plan, "down", 0),
            currentDirectionValue(plan, "up", 0)
        );
    }
    if (totalThroughput) {
        totalThroughput.update(
            currentDirectionValue(summary?.actual_bytes_per_second, "down", 0) * 8,
            currentDirectionValue(summary?.actual_bytes_per_second, "up", 0) * 8
        );
    }
    if (totalRetransmits) {
        totalRetransmits.update(
            retransmitFractionFromSample(summary?.tcp_retransmit_sample?.down) * 100.0,
            retransmitFractionFromSample(summary?.tcp_retransmit_sample?.up) * 100.0
        );
    }
    if (qooGauge !== null) {
        qooGauge.update(summary?.qoo_score);
    }
    updateTopAsnCountBadge();
    updateTrafficCountBadge();
    pushQueuingActivitySample();
}

function applyDeviceLiveData(devices) {
    latestCircuitDevices = devices || [];
    fillLiveDevices(latestCircuitDevices);

    latestCircuitDevices.forEach((device) => {
        const throughputGraph = deviceGraphs["throughputGraph_" + device.device_id];
        if (throughputGraph !== undefined) {
            throughputGraph.update(
                toNumber(device.actual_bytes_per_second?.down, 0) * 8,
                toNumber(device.actual_bytes_per_second?.up, 0) * 8
            );
        }

        const retransmitGraph = deviceGraphs["tcpRetransmitsGraph_" + device.device_id];
        if (retransmitGraph !== undefined) {
            retransmitGraph.update(
                retransmitFractionFromSample(device.tcp_retransmit_sample?.down) * 100.0,
                retransmitFractionFromSample(device.tcp_retransmit_sample?.up) * 100.0
            );
        }
    });
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

function connectCircuitSummaryChannel() {
    channelLink = new DirectChannel({
        CircuitWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        if (msg?.data) {
            applyCircuitSummary(msg.data);
        }
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

function startPingMonitor(circuits) {
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

function stopPingMonitor() {
    if (pinger) {
        wsClient.send({ Private: { StopPingMonitorWatch: null } });
        pinger.close();
        pinger = null;
    }
}

function requestCircuitDevicesSnapshot() {
    if (deviceRequestInFlight) {
        return;
    }
    deviceRequestInFlight = true;
    listenOnce("CircuitDevicesResult", (msg) => {
        deviceRequestInFlight = false;
        if (!msg?.data?.ok || msg.data.circuit_id !== circuit_id) {
            return;
        }
        applyDeviceLiveData(msg.data.devices || []);
    });
    wsClient.send({ CircuitDevices: { circuit: circuit_id } });
}

function requestCircuitFlowSankey() {
    if (sankeyRequestInFlight) {
        return;
    }
    sankeyRequestInFlight = true;
    listenOnce("CircuitFlowSankeyResult", (msg) => {
        sankeyRequestInFlight = false;
        if (msg?.circuit_id !== circuit_id) {
            return;
        }
        latestSankeyFlowMsg = { flows: Array.isArray(msg.flows) ? msg.flows : [] };
        if (isSankeyTabActive()) {
            ensureFlowSankey();
            applyFlowSankeyMessage(latestSankeyFlowMsg);
        } else {
            $("#activeFlowCount").text(getRenderableSankeyFlowCount(latestSankeyFlowMsg));
        }
    });
    wsClient.send({ CircuitFlowSankey: { circuit: circuit_id } });
}

function requestCircuitTopAsns() {
    if (topAsnRequestInFlight) {
        return;
    }
    topAsnRequestInFlight = true;
    listenOnce("CircuitTopAsnsResult", (msg) => {
        topAsnRequestInFlight = false;
        if (msg?.circuit_id !== circuit_id) {
            return;
        }
        latestTopAsnData = msg.data || { total_asns: 0, rows: [] };
        if (isTopAsnTabActive()) {
            renderTopAsnTab();
        } else {
            updateTopAsnCountBadge();
        }
    });
    wsClient.send({
        CircuitTopAsns: {
            query: {
                circuit: circuit_id,
                hide_small: hideSmallFlowsEnabled(),
            },
        },
    });
}

function requestTrafficFlowsPage() {
    if (trafficRequestInFlight) {
        return;
    }
    trafficRequestInFlight = true;
    const query = {
        circuit: circuit_id,
        page: trafficCurrentPage,
        page_size: trafficPageSize,
        hide_small: hideSmallFlowsEnabled(),
        sort_column: trafficSortColumn,
        sort_direction: trafficSortDirection,
    };
    listenOnce("CircuitTrafficFlowsPageResult", (msg) => {
        trafficRequestInFlight = false;
        if (msg?.circuit_id !== circuit_id) {
            return;
        }
        latestTrafficPage = msg.data || null;
        trafficCurrentPage = Math.max(1, toNumber(latestTrafficPage?.query?.page, query.page));
        trafficPageSize = Math.max(1, toNumber(latestTrafficPage?.query?.page_size, query.page_size));
        if (isTrafficTabActive()) {
            renderTrafficTab();
        } else {
            updateTrafficCountBadge();
            updateTrafficPaginationControls();
        }
    });
    wsClient.send({ CircuitTrafficFlowsPage: { query } });
}

function syncCircuitDetailSubscriptions(circuits = null) {
    const pingSourceCircuits = Array.isArray(circuits) && circuits.length > 0 ? circuits : circuitConfigDevices;
    if (isDevicesTabActive()) {
        if (!pinger && Array.isArray(pingSourceCircuits) && pingSourceCircuits.length > 0) {
            startPingMonitor(pingSourceCircuits);
        }
        requestCircuitDevicesSnapshot();
        if (devicePollTimer === null) {
            devicePollTimer = window.setInterval(requestCircuitDevicesSnapshot, 1000);
        }
    } else {
        devicePollTimer = clearPollingTimer(devicePollTimer);
        stopPingMonitor();
    }

    if (isSankeyTabActive()) {
        requestCircuitFlowSankey();
        if (sankeyPollTimer === null) {
            sankeyPollTimer = window.setInterval(requestCircuitFlowSankey, 1000);
        }
    } else {
        sankeyPollTimer = clearPollingTimer(sankeyPollTimer);
    }

    if (isTopAsnTabActive()) {
        requestCircuitTopAsns();
        if (topAsnPollTimer === null) {
            topAsnPollTimer = window.setInterval(requestCircuitTopAsns, 1000);
        }
    } else {
        topAsnPollTimer = clearPollingTimer(topAsnPollTimer);
    }

    if (isTrafficTabActive()) {
        requestTrafficFlowsPage();
        if (trafficPollTimer === null) {
            trafficPollTimer = window.setInterval(requestTrafficFlowsPage, 1000);
        }
    } else {
        trafficPollTimer = clearPollingTimer(trafficPollTimer);
    }
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
            if (isTrafficTabActive()) {
                requestTrafficFlowsPage();
            } else {
                updateTrafficCountBadge();
                updateTrafficPaginationControls();
            }
            if (isTopAsnTabActive()) {
                requestCircuitTopAsns();
            } else {
                updateTopAsnCountBadge();
            }
            if (isSankeyTabActive()) {
                requestCircuitFlowSankey();
            }
        });
    }
    if (pageSize) {
        pageSize.value = String(trafficPageSize);
        pageSize.addEventListener("change", () => {
            const parsed = parseInt(pageSize.value, 10);
            trafficPageSize = Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_TRAFFIC_PAGE_SIZE;
            trafficCurrentPage = 1;
            requestTrafficFlowsPage();
        });
    }
    if (prev) {
        prev.addEventListener("click", () => {
            if (trafficCurrentPage > 1) {
                trafficCurrentPage--;
                requestTrafficFlowsPage();
            }
        });
    }
    if (next) {
        next.addEventListener("click", () => {
            const totalPages = getTrafficTotalPages();
            if (trafficCurrentPage < totalPages) {
                trafficCurrentPage++;
                requestTrafficFlowsPage();
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

let trafficSortColumn = 'rate'; // Default sort by rate
let trafficSortDirection = 'desc'; // 'asc' or 'desc'
let topAsnSortColumn = 'rate';
let topAsnSortDirection = 'desc';
let trafficCurrentPage = 1;
let trafficPageSize = DEFAULT_TRAFFIC_PAGE_SIZE;

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

function truncatedTrafficCell(text, cellClass) {
    const td = document.createElement("td");
    if (cellClass) {
        td.classList.add(cellClass);
    }

    const value = String(text || "");
    td.title = value;

    const span = document.createElement("span");
    span.classList.add("lqos-table-cell-ellipsis");
    span.textContent = value;
    td.appendChild(span);
    return td;
}

function visibleTrafficRows() {
    return Array.isArray(latestTrafficPage?.rows) ? latestTrafficPage.rows : [];
}

function hideSmallFlowsEnabled() {
    return document.getElementById("hideSmallFlows")?.checked ?? false;
}

function getTrafficTotalPages() {
    const totalRows = toNumber(latestTrafficPage?.total_rows, 0);
    return Math.max(1, Math.ceil(totalRows / trafficPageSize));
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
    $("#trafficFlowCount").text(toNumber(latestTrafficPage?.total_rows, latestCircuitSummary?.active_flow_count ?? 0));
}

function updateTopAsnCountBadge() {
    $("#topAsnCount").text(toNumber(latestTopAsnData?.total_asns, latestCircuitSummary?.active_asn_count ?? 0));
}

function sortTopAsnRows(rows) {
    rows.sort((a, b) => {
        const asc = topAsnSortDirection === "asc";
        const normalize = (value) => typeof value === "string" ? value.toLowerCase() : value;
        const totalQoo = (row) => {
            const scores = [row?.qoo_down, row?.qoo_up]
                .map((value) => Number(value))
                .filter((value) => Number.isFinite(value));
            if (!scores.length) {
                return asc ? Number.POSITIVE_INFINITY : Number.NEGATIVE_INFINITY;
            }
            return scores.reduce((sum, value) => sum + value, 0);
        };
        let aVal;
        let bVal;
        switch (topAsnSortColumn) {
            case "asn":
                aVal = normalize(a.asn_name);
                bVal = normalize(b.asn_name);
                break;
            case "country":
                aVal = normalize(a.asn_country);
                bVal = normalize(b.asn_country);
                break;
            case "rtt":
                aVal = toNumber(a.rtt_down_nanos, 0) + toNumber(a.rtt_up_nanos, 0);
                bVal = toNumber(b.rtt_down_nanos, 0) + toNumber(b.rtt_up_nanos, 0);
                break;
            case "qoo":
                aVal = totalQoo(a);
                bVal = totalQoo(b);
                break;
            case "retransmits":
                aVal = a.retransmit_down_pct + a.retransmit_up_pct;
                bVal = b.retransmit_down_pct + b.retransmit_up_pct;
                break;
            case "flows":
                aVal = a.flow_count;
                bVal = b.flow_count;
                break;
            case "rate":
            default:
                aVal = a.down_bps + a.up_bps;
                bVal = b.down_bps + b.up_bps;
                break;
        }
        if (aVal === bVal) {
            return 0;
        }
        return asc ? (aVal < bVal ? -1 : 1) : (aVal > bVal ? -1 : 1);
    });
}

function renderTopAsnTab() {
    const target = document.getElementById("topAsnsTable");
    if (!target) {
        return;
    }

    const displayRows = Array.isArray(latestTopAsnData?.rows) ? latestTopAsnData.rows.slice() : [];
    sortTopAsnRows(displayRows);

    const tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");

    const table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight", "lqos-circuit-traffic-table");
    const thead = document.createElement("thead", "small");
    thead.style.fontSize = "0.8em";

    const createSortableHeader = (text, sortKey, colspan = 1) => {
        const th = theading(text, colspan);
        th.style.cursor = "pointer";
        th.onclick = () => {
            if (topAsnSortColumn === sortKey) {
                topAsnSortDirection = topAsnSortDirection === "asc" ? "desc" : "asc";
            } else {
                topAsnSortColumn = sortKey;
                topAsnSortDirection = sortKey === "qoo" ? "asc" : "desc";
            }
            renderTopAsnTab();
        };
        if (topAsnSortColumn === sortKey) {
            th.innerHTML += topAsnSortDirection === "asc" ? " ▲" : " ▼";
        }
        return th;
    };

    thead.appendChild(createSortableHeader("ASN", "asn"));
    thead.appendChild(createSortableHeader("Country", "country"));
    thead.appendChild(createSortableHeader("Current Rate (d/u)", "rate", 2));
    thead.appendChild(createSortableHeader("RTT (d/u)", "rtt", 2));
    thead.appendChild(createSortableHeader("QoO (d/u)", "qoo", 2));
    thead.appendChild(createSortableHeader("TCP rxmit (d/u)", "retransmits", 2));
    thead.appendChild(createSortableHeader("Flows", "flows"));
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    if (displayRows.length === 0) {
        const empty = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 11;
        td.classList.add("text-center", "text-muted", "small");
        td.textContent = "No recent ASN activity available for this circuit.";
        empty.appendChild(td);
        tbody.appendChild(empty);
    } else {
        displayRows.forEach((rowData) => {
            const row = document.createElement("tr");
            row.classList.add("small");

            row.appendChild(truncatedTrafficCell(rowData.asn_name, "lqos-circuit-traffic-asn-cell"));
            row.appendChild(truncatedTrafficCell(rowData.asn_country, "lqos-circuit-traffic-country-cell"));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.down_bps, plan.down)));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.up_bps, plan.up)));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rtt_down_nanos)));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rtt_up_nanos)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qoo_down)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qoo_up)));
            row.appendChild(simpleRowHtml(rowData.retransmit_down_pct > 0 ? formatRetransmitFraction(rowData.retransmit_down_pct) : "-"));
            row.appendChild(simpleRowHtml(rowData.retransmit_up_pct > 0 ? formatRetransmitFraction(rowData.retransmit_up_pct) : "-"));
            row.appendChild(simpleRow(scaleNumber(rowData.flow_count)));

            tbody.appendChild(row);
        });
    }

    table.appendChild(tbody);
    tableWrap.appendChild(table);
    clearDiv(target);
    target.appendChild(tableWrap);
    updateTopAsnCountBadge();
}

function renderTrafficTab() {
    const target = document.getElementById("allTraffic");
    if (!target) {
        return;
    }

    const visibleRows = visibleTrafficRows().slice();
    const totalPages = Math.max(1, Math.ceil(toNumber(latestTrafficPage?.total_rows, 0) / trafficPageSize));
    trafficCurrentPage = Math.min(Math.max(1, trafficCurrentPage), totalPages);
    const pagedRows = visibleRows;

    let tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");

    let table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight", "lqos-circuit-traffic-table");
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
            requestTrafficFlowsPage();
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
            row.style.opacity = toNumber(rowData.opacity, 1);

            row.appendChild(truncatedTrafficCell(rowData.protocol_name, "lqos-circuit-traffic-protocol-cell"));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.down_bps, plan.down)));
            row.appendChild(simpleRowHtml(formatThroughput(rowData.up_bps, plan.up)));
            row.appendChild(simpleRow(scaleNumber(rowData.bytes_sent_down)));
            row.appendChild(simpleRow(scaleNumber(rowData.bytes_sent_up)));
            row.appendChild(simpleRow(scaleNumber(rowData.packets_sent_down)));
            row.appendChild(simpleRow(scaleNumber(rowData.packets_sent_up)));
            row.appendChild(simpleRowHtml(rowData.retransmit_down_pct > 0 ? formatRetransmitFraction(rowData.retransmit_down_pct) : "-"));
            row.appendChild(simpleRowHtml(rowData.retransmit_up_pct > 0 ? formatRetransmitFraction(rowData.retransmit_up_pct) : "-"));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rtt_down_nanos)));
            row.appendChild(simpleRowHtml(formatRttNanos(rowData.rtt_up_nanos)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qoo_down)));
            row.appendChild(simpleRowHtml(formatQooScore(rowData.qoo_up)));
            row.appendChild(truncatedTrafficCell(rowData.asn_name, "lqos-circuit-traffic-asn-cell"));
            row.appendChild(truncatedTrafficCell(rowData.asn_country, "lqos-circuit-traffic-country-cell"));
            row.appendChild(simpleRow(rowData.remote_ip));

            const td = document.createElement("td");
            td.classList.add("text-center");
            const button = document.createElement("button");
            button.type = "button";
            button.className = "btn btn-outline-secondary btn-sm flow-rtt-exclude-btn";
            button.textContent = "Exclude";
            button.disabled = !rowData.remote_ip;
            button.title = "Open a wizard to exclude RTT samples for this remote IP/CIDR (requires saving in Flow Tracking config).";
            button.dataset.remoteIp = rowData.remote_ip;
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
                toNumber(device.actual_bytes_per_second?.down, 0) * 8,
                toNumber(device.plan?.down, 0)
            );
        }

        if (throughputUp !== null) {
            throughputUp.innerHTML = formatThroughput(
                toNumber(device.actual_bytes_per_second?.up, 0) * 8,
                toNumber(device.plan?.up, 0)
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
            tcp_retransmitsDown.innerHTML = formatRetransmitFraction(
                retransmitFractionFromSample(device.tcp_retransmit_sample?.down)
            );
        }

        if (tcp_retransmitsUp !== null) {
            tcp_retransmitsUp.innerHTML = formatRetransmitFraction(
                retransmitFractionFromSample(device.tcp_retransmit_sample?.up)
            );
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

        let card = document.createElement("div");
        card.classList.add("lqos-circuit-device-card");
        outer.appendChild(card);

        let row = document.createElement("div");
        row.classList.add("row", "g-2");
        card.appendChild(row);

        let d = document.createElement("div");
        d.classList.add("col-12", "col-xl-5", "col-xxl-4", "lqos-circuit-device-summary");
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
        graphCol.classList.add("col-12", "col-xl-7", "col-xxl-8", "lqos-circuit-device-graphs");
        row.appendChild(graphCol);

        let graphRow = document.createElement("div");
        graphRow.classList.add("row", "g-2", "lqos-circuit-device-graphs-row");
        graphCol.appendChild(graphRow);

        function addGraph(divId, graphFactory) {
            let col = document.createElement("div");
            col.classList.add("col-12", "col-md-6");
            let div = document.createElement("div");
            div.id = divId;
            div.classList.add("lqos-circuit-device-graph");
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
    funnelParentNodeName = parentNode?.name || null;
    funnelParentNodeId = parentNode?.id || null;
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
    if (msg.event !== "NetworkTreeLite" || (!funnelParentNodeName && !funnelParentNodeId)) {
        return;
    }

    const state = resolveFunnelState(msg, {
        id: funnelParentNodeId,
        name: funnelParentNodeName,
    });
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
        const packetsDown = retransmitPacketsForNode(myMessage, 0);
        const packetsUp = retransmitPacketsForNode(myMessage, 1);
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
                            requestCircuitById((payload) => {
                                wireupAnalysis(payload.devices || []);
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
    requestCircuitById((payload) => {
        const circuits = payload.devices || [];
        const advisory = payload.ethernet_advisory || null;
        let circuit = circuits[0];
        const parentNode = resolveCircuitParentNode(payload, circuits);
        circuitConfigDevices = circuits;
        $("#circuitName").text(circuit.circuit_name);
        $("#circuitName").attr("title", circuit.circuit_name || "");
        applyParentNodeLink(parentNode?.name || "");
        $("#bwMax").text(formatPlanSpeedPair(circuit.download_max_mbps, circuit.upload_max_mbps));
        $("#bwMin").text(formatPlanSpeedPair(circuit.download_min_mbps, circuit.upload_min_mbps));
        renderEthernetAdvisory(advisory);
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
        initTabLifecycle(parentNode);
        updateQueuingActivityCards();

        connectCircuitSummaryChannel();
        subscribeToCake();
        wireupAnalysis(circuits);
    }, () => {
        alert("Circuit with id " + circuit_id + " not found");
    });
}

function cleanupCircuitPage() {
    if (channelLink) {
        wsClient.send({ Private: { StopCircuitWatcher: null } });
        channelLink.close();
        channelLink = null;
    }
    if (cakeChannel) {
        cakeChannel.close();
        cakeChannel = null;
    }
    if (pinger) {
        stopPingMonitor();
    }
    devicePollTimer = clearPollingTimer(devicePollTimer);
    sankeyPollTimer = clearPollingTimer(sankeyPollTimer);
    topAsnPollTimer = clearPollingTimer(topAsnPollTimer);
    trafficPollTimer = clearPollingTimer(trafficPollTimer);
    if (funnelSubscription) {
        funnelSubscription.dispose();
        funnelSubscription = null;
    }
    funnelInitialized = false;
    funnelParentNodeName = null;
    funnelParentNodeId = null;
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
