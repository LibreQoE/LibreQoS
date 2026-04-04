import {clearDiv, clientTableHeader, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {
    formatCakeStat, formatCakeStatPercent,
    formatRetransmit, formatRetransmitFraction, retransmitFractionFromSample,
    formatRtt,
    formatThroughput,
} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {QooScoreGauge} from "./graphs/qoo_score_gauge";
import {enableTooltipsWithin} from "./lq_js_common/helpers/tooltips";
import {scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {get_ws_client, subscribeWS} from "./pubsub/ws";

var tree = null;
var parent = 0;
var upParent = 0;
var subscribed = false;
var expandedNodes = new Set();
var childrenByParentId = new Map();
var stormguardNodes = new Set();
var treeBitsGauge = null;
var treeQooGauge = null;
var lastAttachedCircuitsPage = null;
var attachedCircuitsWatchSignature = null;
var currentSelectionIdentity = null;
var rootGaugeConfigMax = null;
var selectionLocator = {
    nodeId: null,
    nodePath: null,
    lastKnownIndex: 0,
};
var nodeRateOverrideState = {
    loading: false,
    saving: false,
    data: null,
    error: null,
    flash: null,
};
var nodeTopologyOverrideState = {
    loading: false,
    saving: false,
    data: null,
    error: null,
    flash: null,
};
var nodeOverrideInputsDirty = false;
var nodeOverrideLastSeedSignature = null;
var nodeTopologyInputsDirty = false;
var nodeTopologyLastSeedSignature = null;
const wsClient = get_ws_client();
const QOO_TOOLTIP_HTML = "<h5>Quality of Outcome (QoO)</h5>" +
    "<p>Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).<br>" +
    "https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/<br>" +
    "LibreQoS implements a latency and loss-based model to estimate quality of outcome.</p>";
const THROUGHPUT_COMPARE_EPSILON_MBPS = 0.01;
const NODE_OVERRIDE_PENDING_TOOLTIP = "Stored as an operator override. Will be applied to generated network.json on the next scheduler run.";
const TOPOLOGY_OVERRIDE_PENDING_TOOLTIP = "Stored as an operator topology override. The selected parent will be applied on the next scheduler run.";

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

function sendWsRequest(responseEvent, request) {
    return new Promise((resolve, reject) => {
        let done = false;
        const responseHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            resolve(msg);
        };
        const errorHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            reject(msg);
        };
        wsClient.on(responseEvent, responseHandler);
        wsClient.on("Error", errorHandler);
        wsClient.send(request);
    });
}

function sendPrivateRequest(command) {
    wsClient.send({Private: command});
}

async function loadRootGaugeConfigMax() {
    try {
        const msg = await sendWsRequest("GetConfig", {GetConfig: {}});
        const queues = msg?.data?.queues;
        if (!queues) {
            return;
        }
        rootGaugeConfigMax = [
            toNumber(queues.downlink_bandwidth_mbps, 0),
            toNumber(queues.uplink_bandwidth_mbps, 0),
        ];
        const node = currentNode();
        if (node && isSyntheticRootNode(node)) {
            updateTreeGauges(node);
        }
    } catch (_error) {
        // If config load fails, keep the existing node-derived gauge behavior.
    }
}

function formatDeviceIp(ip) {
    if (typeof ip === "string") {
        return ip;
    }
    if (ip === null || ip === undefined) {
        return "-";
    }
    if (ip instanceof Uint8Array) {
        return formatIpBytes(ip);
    }
    if (Array.isArray(ip)) {
        if (ip.every((entry) => Number.isFinite(Number(entry)))) {
            return formatIpBytes(ip);
        }
        return ip.map((entry) => formatDeviceIp(entry)).filter(Boolean).join(", ");
    }
    if (typeof ip === "object") {
        if (ip.V4 !== undefined) {
            return formatDeviceIp(ip.V4);
        }
        if (ip.V6 !== undefined) {
            return formatDeviceIp(ip.V6);
        }
        if (ip.addr !== undefined) {
            return formatDeviceIp(ip.addr);
        }
        if (Array.isArray(ip.data) || ip.data instanceof Uint8Array) {
            return formatDeviceIp(ip.data);
        }
    }
    return String(ip);
}

function formatIpBytes(bytes) {
    const list = Array.from(bytes);
    if (list.length === 4) {
        return list.join(".");
    }
    if (list.length === 16) {
        const parts = [];
        for (let i = 0; i < list.length; i += 2) {
            const part = ((Number(list[i]) || 0) << 8) | (Number(list[i + 1]) || 0);
            parts.push(part.toString(16).padStart(4, "0"));
        }
        return parts.join(":");
    }
    return list.join(".");
}

function abbreviateIpForTable(ipText) {
    if (typeof ipText !== "string") {
        return String(ipText ?? "");
    }
    if (!ipText.includes(":")) {
        return ipText;
    }
    const groups = ipText.split(":").filter((group) => group.length > 0);
    if (groups.length <= 3) {
        return ipText;
    }
    return `${groups.slice(0, 3).join(":")}:...`;
}

function summarizeIpListForTable(ipList) {
    const displayList = ipList.map((entry) => abbreviateIpForTable(entry));
    if (displayList.length > 1) {
        return `${displayList[0]} +${displayList.length - 1}`;
    }
    return displayList.join(", ");
}

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

function ethernetCapsPageHref(badge) {
    const tier = encodeURIComponent(formatEthernetPortLabel(badge?.negotiated_ethernet_mbps));
    return `/ethernet_caps.html?tier=${tier}`;
}

function formatEthernetTooltip(badge) {
    if (!badge) {
        return "";
    }
    return `Requested plan ${toNumber(badge.requested_download_mbps, 0)} / ${toNumber(badge.requested_upload_mbps, 0)} Mbps exceeded detected Ethernet speed. ` +
        `Shaping auto-capped to ${toNumber(badge.applied_download_mbps, 0)} / ${toNumber(badge.applied_upload_mbps, 0)} Mbps.`;
}

function ethernetBadgeClass(tierLabel) {
    if (tierLabel === "10M") return "text-bg-danger";
    if (tierLabel === "100M") return "text-bg-warning";
    return "text-bg-info";
}

function configuredMax(node) {
    return node.configured_max_throughput || node.max_throughput || [0, 0];
}

function effectiveMax(node) {
    return node.effective_max_throughput || configuredMax(node);
}

function rootGaugeMaxAvailable() {
    return Array.isArray(rootGaugeConfigMax)
        && rootGaugeConfigMax.length === 2
        && Number.isFinite(Number(rootGaugeConfigMax[0]))
        && Number.isFinite(Number(rootGaugeConfigMax[1]));
}

function nodeSnapshotGaugeMax(node) {
    if (isSyntheticRootNode(node) && rootGaugeMaxAvailable()) {
        return [
            toNumber(rootGaugeConfigMax[0], 0),
            toNumber(rootGaugeConfigMax[1], 0),
        ];
    }
    return effectiveMax(node);
}

function ratesMatch(a, b) {
    return Math.abs(toNumber(a?.[0], 0) - toNumber(b?.[0], 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS
        && Math.abs(toNumber(a?.[1], 0) - toNumber(b?.[1], 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS;
}

function formatLimitValue(mbps) {
    if (toNumber(mbps, 0) === 0) {
        return "Unlimited";
    }
    return scaleNumber(toNumber(mbps, 0) * 1000 * 1000, 1);
}

function formatMbpsInputValue(mbps) {
    if (mbps === null || mbps === undefined) {
        return "";
    }
    const numeric = Number(mbps);
    if (!Number.isFinite(numeric)) {
        return "";
    }
    return numeric.toFixed(2).replace(/\.?0+$/, "");
}

function ratesApproximatelyEqual(left, right) {
    return Math.abs(toNumber(left, 0) - toNumber(right, 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS;
}

function formatRatePair(downMbps, upMbps) {
    return `${formatLimitValue(downMbps)} / ${formatLimitValue(upMbps)}`;
}

function isSyntheticRootNode(node) {
    return !!node
        && (node.id === null || node.id === undefined)
        && node.immediate_parent === null
        && node.name === "Root";
}

function currentNode() {
    return tree && tree[parent] ? tree[parent][1] : null;
}

function currentNodeQuery() {
    const node = currentNode();
    if (!node) {
        return null;
    }
    const query = {
        node_name: node.name,
    };
    if (node.id) {
        query.node_id = node.id;
    }
    return query;
}

function currentNodeRateQuery() {
    return currentNodeQuery();
}

function currentNodeTopologyQuery() {
    return currentNodeQuery();
}

function buildEffectiveLimitCellHtml(node) {
    const effective = effectiveMax(node);
    const effectiveValue = `${formatLimitValue(effective[0])} / ${formatLimitValue(effective[1])}`;
    return `<div class="lqos-limit-block"><div class="lqos-limit-line">${effectiveValue}</div></div>`;
}

function buildConfiguredLimitCellHtml(node) {
    const configured = configuredMax(node);
    const effective = effectiveMax(node);
    const secondaryClass = ratesMatch(effective, configured)
        ? "lqos-limit-line lqos-limit-secondary is-match"
        : "lqos-limit-line lqos-limit-secondary";
    const configuredValue = `${formatLimitValue(configured[0])} / ${formatLimitValue(configured[1])}`;
    return `<div class="lqos-limit-block"><div class="${secondaryClass}">${configuredValue}</div></div>`;
}

function buildDirectionalLimitHtml(primaryMbps, configuredMbps, showConfigured) {
    const primary = formatLimitValue(primaryMbps);
    if (!showConfigured) {
        return `<div class="lqos-limit-block"><div class="lqos-limit-line">${primary}</div></div>`;
    }
    const match = Math.abs(toNumber(primaryMbps, 0) - toNumber(configuredMbps, 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS;
    const secondaryClass = match
        ? "lqos-limit-line lqos-limit-secondary is-match"
        : "lqos-limit-line lqos-limit-secondary";
    return `<div class="lqos-limit-block"><div class="lqos-limit-line">${primary}</div><div class="${secondaryClass}">Cfg ${formatLimitValue(configuredMbps)}</div></div>`;
}

function subtreeSiteCount(node) {
    return toNumber(node.subtree_site_count, 0);
}

function subtreeCircuitCount(node) {
    return toNumber(node.subtree_circuit_count, 0);
}

function branchSizeParts(node) {
    const parts = [];
    const circuitCount = subtreeCircuitCount(node);
    if (circuitCount > 0) {
        parts.push(`${circuitCount} ${circuitCount === 1 ? "circuit" : "circuits"}`);
    }
    const siteCount = subtreeSiteCount(node);
    if (siteCount > 0) {
        parts.push(`${siteCount} ${siteCount === 1 ? "site" : "sites"}`);
    }
    return parts;
}

function formatBranchSize(node) {
    const parts = branchSizeParts(node);
    if (parts.length === 0) {
        return "No descendant sites or circuits";
    }
    return parts.join(" / ");
}

function buildStatusIcon(iconName, textClass, title) {
    let icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconName, textClass);
    icon.setAttribute("data-bs-toggle", "tooltip");
    icon.setAttribute("data-bs-placement", "top");
    icon.setAttribute("title", title);
    return icon;
}

function isStormguardNode(node) {
    return node && typeof node.name === "string" && stormguardNodes.has(node.name);
}

function renderHeaderStatusIcons(node) {
    const target = document.getElementById("nodeStateBadges");
    if (!target) {
        return;
    }
    clearDiv(target);
    if (!node) {
        return;
    }
    if (node.virtual === true) {
        target.appendChild(buildStatusIcon(
            "fa-ghost",
            "text-secondary",
            "Virtual node (logical only; not shaped in HTB)."
        ));
    }
    if (node.runtime_virtualized === true) {
        target.appendChild(buildStatusIcon(
            "fa-layer-group",
            "text-warning",
            "TreeGuard runtime-virtualized node (physically bypassed in Bakery while preserved in the logical hierarchy)."
        ));
    }
    if (isStormguardNode(node)) {
        target.appendChild(buildStatusIcon(
            "fa-cloud-bolt",
            "text-primary",
            "StormGuard-managed node (dynamic queue limits active)."
        ));
    }
    enableTooltipsWithin(target);
}

function sameStringSet(a, b) {
    if (a.size !== b.size) {
        return false;
    }
    for (const entry of a) {
        if (!b.has(entry)) {
            return false;
        }
    }
    return true;
}

function normalizeNodePath(path) {
    if (!Array.isArray(path)) {
        return null;
    }
    const normalized = path
        .map((entry) => typeof entry === "string" ? entry : String(entry ?? ""))
        .filter((entry) => entry.length > 0);
    return normalized.length > 0 ? normalized : null;
}

function parseNodePathParam(rawPath) {
    if (typeof rawPath !== "string" || rawPath.length === 0) {
        return null;
    }
    try {
        return normalizeNodePath(JSON.parse(rawPath));
    } catch (_error) {
        return null;
    }
}

function pathEquals(left, right) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
        return false;
    }
    for (let i = 0; i < left.length; i++) {
        if (left[i] !== right[i]) {
            return false;
        }
    }
    return true;
}

function getNodePathNames(nodeId, node = tree?.[nodeId]?.[1]) {
    if (!node) {
        return null;
    }
    if (isSyntheticRootNode(node)) {
        return ["Root"];
    }
    const parentIndexes = Array.isArray(node.parents) && node.parents.length > 0
        ? node.parents
        : [nodeId];
    const names = [];
    for (const idx of parentIndexes) {
        const entry = tree?.[idx];
        if (!entry || !entry[1] || typeof entry[1].name !== "string" || entry[1].name.length === 0) {
            return normalizeNodePath([node.name]);
        }
        names.push(entry[1].name);
    }
    if (names.length === 0 || names[names.length - 1] !== node.name) {
        names.push(node.name);
    }
    return normalizeNodePath(names);
}

function selectionIdentityForNode(nodeId, node = tree?.[nodeId]?.[1]) {
    if (!node) {
        return null;
    }
    if (typeof node.id === "string" && node.id.length > 0) {
        return `id:${node.id}`;
    }
    const nodePath = getNodePathNames(nodeId, node);
    if (nodePath) {
        return `path:${JSON.stringify(nodePath)}`;
    }
    return `index:${nodeId}`;
}

function syncBrowserUrlToSelection() {
    if (typeof window === "undefined" || typeof history === "undefined") {
        return;
    }
    const url = new URL(window.location.href);
    url.searchParams.set("parent", String(parent));
    if (selectionLocator.nodeId) {
        url.searchParams.set("nodeId", selectionLocator.nodeId);
    } else {
        url.searchParams.delete("nodeId");
    }
    if (selectionLocator.nodePath) {
        url.searchParams.set("nodePath", JSON.stringify(selectionLocator.nodePath));
    } else {
        url.searchParams.delete("nodePath");
    }
    const nextUrl = `${url.pathname}?${url.searchParams.toString()}${url.hash}`;
    const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (nextUrl !== currentUrl) {
        history.replaceState(null, "", nextUrl);
    }
}

function rememberSelection(nodeId, node = tree?.[nodeId]?.[1]) {
    if (!node) {
        return false;
    }
    const nextIdentity = selectionIdentityForNode(nodeId, node);
    const identityChanged = currentSelectionIdentity !== nextIdentity;
    currentSelectionIdentity = nextIdentity;
    selectionLocator.nodeId = typeof node.id === "string" && node.id.length > 0 ? node.id : null;
    selectionLocator.nodePath = getNodePathNames(nodeId, node);
    selectionLocator.lastKnownIndex = nodeId;
    syncBrowserUrlToSelection();
    return identityChanged;
}

function findNodeIndexByStableId(nodeId) {
    if (!tree || typeof nodeId !== "string" || nodeId.length === 0) {
        return null;
    }
    for (let i = 0; i < tree.length; i++) {
        const node = tree[i]?.[1];
        if (node?.id === nodeId) {
            return i;
        }
    }
    return null;
}

function findNodeIndexByPath(nodePath) {
    if (!tree || !Array.isArray(nodePath) || nodePath.length === 0) {
        return null;
    }
    for (let i = 0; i < tree.length; i++) {
        const node = tree[i]?.[1];
        if (!node) {
            continue;
        }
        if (pathEquals(getNodePathNames(i, node), nodePath)) {
            return i;
        }
    }
    return null;
}

function resolveSelectedParentIndex() {
    const byStableId = findNodeIndexByStableId(selectionLocator.nodeId);
    if (byStableId !== null) {
        return byStableId;
    }
    const byPath = findNodeIndexByPath(selectionLocator.nodePath);
    if (byPath !== null) {
        return byPath;
    }
    if (Number.isInteger(selectionLocator.lastKnownIndex) && tree?.[selectionLocator.lastKnownIndex] !== undefined) {
        return selectionLocator.lastKnownIndex;
    }
    if (tree?.[parent] !== undefined) {
        return parent;
    }
    if (tree?.[0] !== undefined) {
        return 0;
    }
    return null;
}

function reconcileSelection() {
    const resolvedParent = resolveSelectedParentIndex();
    if (resolvedParent === null) {
        return {parentChanged: false, identityChanged: false};
    }
    const previousParent = parent;
    parent = resolvedParent;
    const identityChanged = rememberSelection(parent);
    return {
        parentChanged: previousParent !== parent,
        identityChanged,
    };
}

function treeHref(nodeId, node = tree?.[nodeId]?.[1]) {
    const params = new URLSearchParams();
    params.set("parent", String(nodeId));
    if (node?.id) {
        params.set("nodeId", node.id);
    }
    const nodePath = getNodePathNames(nodeId, node);
    if (nodePath) {
        params.set("nodePath", JSON.stringify(nodePath));
    }
    return `/tree.html?${params.toString()}`;
}

function formatTreeCount(value) {
    const numeric = toNumber(value, 0);
    return numeric > 0 ? String(numeric) : "-";
}

function buildTreeRowStatusIcon(iconName, isActive, activeTitle, inactiveTitle) {
    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconName, "lqos-tree-row-status-icon");
    if (!isActive) {
        icon.classList.add("is-inactive");
    }
    icon.setAttribute("data-bs-toggle", "tooltip");
    icon.setAttribute("data-bs-placement", "top");
    icon.setAttribute("title", isActive ? activeTitle : inactiveTitle);
    return icon;
}

function renderBreadcrumb() {
    const target = document.getElementById("treeBreadcrumb");
    if (!target) {
        return;
    }
    clearDiv(target);
    if (!tree || tree[parent] === undefined) {
        return;
    }

    const current = tree[parent][1];
    const path = Array.isArray(current.parents) && current.parents.length > 0
        ? current.parents
        : [parent];
    const ancestors = path.slice(0, -1);
    const navWrap = document.createElement("div");
    navWrap.classList.add("lqos-tree-nav");
    const trail = document.createElement("div");
    trail.classList.add("lqos-tree-breadcrumb");

    ancestors.forEach((nodeId, index) => {
        const entry = tree[nodeId];
        if (!entry || !entry[1]) {
            return;
        }
        if (index > 0) {
            const separator = document.createElement("span");
            separator.classList.add("lqos-tree-breadcrumb-separator");
            separator.innerHTML = "<i class='fa fa-chevron-right'></i>";
            trail.appendChild(separator);
        }
        const link = document.createElement("a");
        link.href = treeHref(nodeId, entry[1]);
        link.classList.add("lqos-tree-breadcrumb-link", "redactable");
        link.textContent = entry[1].name;
        trail.appendChild(link);
    });

    if (ancestors.length > 0) {
        const separator = document.createElement("span");
        separator.classList.add("lqos-tree-breadcrumb-separator");
        separator.innerHTML = "<i class='fa fa-chevron-right'></i>";
        trail.appendChild(separator);
    }

    const currentNode = document.createElement("span");
    currentNode.classList.add("lqos-tree-breadcrumb-current", "redactable");
    currentNode.textContent = current.name;
    trail.appendChild(currentNode);

    navWrap.appendChild(trail);
    target.appendChild(navWrap);
}

function renderContextMeta(node) {
    const target = document.getElementById("treeContextMeta");
    if (!target) {
        return;
    }
    clearDiv(target);
    if (!node) {
        return;
    }

    const metaItems = [];
    if (node.type) {
        metaItems.push(node.type);
    }
    const branchSize = formatBranchSize(node);
    if (branchSize) {
        metaItems.push(branchSize);
    }
    metaItems.push(`Max ${formatRatePair(...effectiveMax(node))}`);

    metaItems.forEach((text) => {
        const pill = document.createElement("span");
        pill.classList.add("lqos-tree-context-pill");
        pill.textContent = text;
        target.appendChild(pill);
    });
}

function setNodeOverrideFlash(message, variant = "success") {
    nodeRateOverrideState.flash = message ? {message, variant} : null;
    renderNodeSettings();
}

function setNodeTopologyOverrideFlash(message, variant = "success") {
    nodeTopologyOverrideState.flash = message ? {message, variant} : null;
    renderNodeSettings();
}

function renderAlertMessages(targetId, messages, variant) {
    const target = document.getElementById(targetId);
    if (!target) {
        return;
    }
    clearDiv(target);
    messages.forEach((message) => {
        if (!message) {
            return;
        }
        const alert = document.createElement("div");
        alert.classList.add("alert", `alert-${variant}`, "py-2", "px-3", "small");
        alert.textContent = message;
        target.appendChild(alert);
    });
}

function escapeHtml(text) {
    return String(text)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;")
        .replaceAll("'", "&#39;");
}

function renderCompatibilityWarningsIndicator(messages) {
    const target = document.getElementById("nodeOverrideCompatibilityIndicator");
    if (!target) {
        return;
    }
    const visibleMessages = Array.isArray(messages) ? messages.filter(Boolean) : [];
    const nextSignature = JSON.stringify(visibleMessages);
    if (target.dataset.warningSignature === nextSignature) {
        return;
    }
    target.dataset.warningSignature = nextSignature;
    clearDiv(target);
    if (visibleMessages.length === 0) {
        return;
    }
    const tooltipHtml = visibleMessages
        .map((message) => `<div>${escapeHtml(message)}</div>`)
        .join("");
    const icon = document.createElement("span");
    icon.classList.add("lqos-tree-compat-indicator");
    icon.setAttribute("data-bs-toggle", "tooltip");
    icon.setAttribute("data-bs-placement", "top");
    icon.setAttribute("data-bs-html", "true");
    icon.setAttribute("title", tooltipHtml);
    icon.setAttribute("aria-label", `Compatibility warnings: ${visibleMessages.length}`);
    icon.innerHTML = `<i class="fa fa-triangle-exclamation"></i><span class="lqos-tree-compat-count">${visibleMessages.length}</span>`;
    target.appendChild(icon);
}

function currentOverridePair(node, overrideData) {
    const base = configuredMax(node);
    return [
        overrideData && overrideData.has_override && overrideData.override_download_bandwidth_mbps !== null && overrideData.override_download_bandwidth_mbps !== undefined
            ? overrideData.override_download_bandwidth_mbps
            : base[0],
        overrideData && overrideData.has_override && overrideData.override_upload_bandwidth_mbps !== null && overrideData.override_upload_bandwidth_mbps !== undefined
            ? overrideData.override_upload_bandwidth_mbps
            : base[1],
    ];
}

function isPendingApply(node, overrideData) {
    if (!node || !overrideData || !overrideData.has_override) {
        return false;
    }
    const configured = configuredMax(node);
    if (overrideData.override_download_bandwidth_mbps !== null
        && overrideData.override_download_bandwidth_mbps !== undefined
        && !ratesApproximatelyEqual(overrideData.override_download_bandwidth_mbps, configured[0])) {
        return true;
    }
    if (overrideData.override_upload_bandwidth_mbps !== null
        && overrideData.override_upload_bandwidth_mbps !== undefined
        && !ratesApproximatelyEqual(overrideData.override_upload_bandwidth_mbps, configured[1])) {
        return true;
    }
    return false;
}

function setNodeOverrideInputsDisabled(disabled) {
    ["nodeOverrideDownload", "nodeOverrideUpload", "nodeOverrideSave", "nodeOverrideClear"].forEach((id) => {
        const el = document.getElementById(id);
        if (el) {
            el.disabled = disabled;
        }
    });
}

function maybeSeedOverrideInputs(node, overrideData, force = false) {
    const downloadInput = document.getElementById("nodeOverrideDownload");
    const uploadInput = document.getElementById("nodeOverrideUpload");
    if (!downloadInput || !uploadInput || !node) {
        return;
    }

    const [down, up] = currentOverridePair(node, overrideData);
    const signature = JSON.stringify({
        nodeId: node.id ?? null,
        nodeName: node.name,
        download: down,
        upload: up,
    });
    if (!force && nodeOverrideInputsDirty) {
        return;
    }
    if (!force && nodeOverrideLastSeedSignature === signature) {
        return;
    }

    downloadInput.value = formatMbpsInputValue(down);
    uploadInput.value = formatMbpsInputValue(up);
    nodeOverrideLastSeedSignature = signature;
    nodeOverrideInputsDirty = false;
}

function renderOverrideValue(node, overrideData) {
    const target = document.getElementById("nodeSettingsOverride");
    if (!target) {
        return;
    }
    target.textContent = "";
    clearDiv(target);
    const wrap = document.createElement("span");
    wrap.classList.add("lqos-tree-detail-value");

    const value = document.createElement("span");
    if (!overrideData || !overrideData.has_override) {
        value.textContent = "- / -";
        wrap.appendChild(value);
        target.appendChild(wrap);
        return;
    }

    const [overrideDown, overrideUp] = currentOverridePair(node, overrideData);
    value.textContent = formatRatePair(overrideDown, overrideUp);
    wrap.appendChild(value);

    if (isPendingApply(node, overrideData)) {
        const pending = document.createElement("span");
        pending.classList.add("lqos-tree-pending");
        pending.setAttribute("data-bs-toggle", "tooltip");
        pending.setAttribute("data-bs-placement", "top");
        pending.setAttribute("title", NODE_OVERRIDE_PENDING_TOOLTIP);

        const symbol = document.createElement("span");
        symbol.classList.add("lqos-tree-pending-symbol");
        symbol.textContent = "⟳";
        pending.appendChild(symbol);

        const label = document.createElement("span");
        label.textContent = "Pending";
        pending.appendChild(label);
        wrap.appendChild(pending);
    }

    target.appendChild(wrap);
}

function topologyOverrideLabel() {
    return "Pinned Parent";
}

function overrideParentNameMap(overrideData) {
    const names = new Map();
    const ids = Array.isArray(overrideData?.override_parent_node_ids)
        ? overrideData.override_parent_node_ids
        : [];
    const labels = Array.isArray(overrideData?.override_parent_node_names)
        ? overrideData.override_parent_node_names
        : [];
    ids.forEach((id, index) => {
        if (typeof id === "string" && id.length > 0) {
            names.set(id, labels[index] || id);
        }
    });
    return names;
}

function candidateNameById(overrideData) {
    const names = overrideParentNameMap(overrideData);
    const candidates = Array.isArray(overrideData?.candidate_parents)
        ? overrideData.candidate_parents
        : [];
    candidates.forEach((candidate) => {
        if (candidate?.node_id) {
            names.set(candidate.node_id, candidate.node_name || candidate.node_id);
        }
    });
    return names;
}

function resolvedOverrideParentId(overrideData) {
    if (!overrideData?.has_override) {
        return null;
    }
    const savedIds = Array.isArray(overrideData.override_parent_node_ids)
        ? overrideData.override_parent_node_ids.filter((id) => typeof id === "string" && id.length > 0)
        : [];
    if (savedIds.length === 0) {
        return null;
    }
    return savedIds[0] || null;
}

function isTopologyPendingApply(overrideData) {
    if (!overrideData?.has_override) {
        return false;
    }
    const currentParentId = overrideData.current_parent_node_id || null;
    const desiredParentId = resolvedOverrideParentId(overrideData);
    return !!desiredParentId && desiredParentId !== currentParentId;
}

function renderTopologyOverrideValue(overrideData) {
    const target = document.getElementById("nodeTopologyOverrideValue");
    if (!target) {
        return;
    }
    clearDiv(target);
    const wrap = document.createElement("span");
    wrap.classList.add("lqos-tree-detail-value");

    const value = document.createElement("span");
    if (!overrideData?.has_override) {
        value.textContent = "None";
        wrap.appendChild(value);
        target.appendChild(wrap);
        return;
    }

    const nameById = candidateNameById(overrideData);
    const parentNames = (overrideData.override_parent_node_ids || [])
        .map((id) => nameById.get(id) || id)
        .filter(Boolean);
    value.textContent = `Pinned: ${parentNames[0] || "-"}`;
    wrap.appendChild(value);

    if (isTopologyPendingApply(overrideData)) {
        const pending = document.createElement("span");
        pending.classList.add("lqos-tree-pending");
        pending.setAttribute("data-bs-toggle", "tooltip");
        pending.setAttribute("data-bs-placement", "top");
        pending.setAttribute("title", TOPOLOGY_OVERRIDE_PENDING_TOOLTIP);

        const symbol = document.createElement("span");
        symbol.classList.add("lqos-tree-pending-symbol");
        symbol.textContent = "⟳";
        pending.appendChild(symbol);

        const label = document.createElement("span");
        label.textContent = "Pending";
        pending.appendChild(label);
        wrap.appendChild(pending);
    }

    target.appendChild(wrap);
}

function setNodeTopologyInputsDisabled(disabled) {
    ["nodeTopologyPinnedParent", "nodeTopologySave", "nodeTopologyClear"].forEach((id) => {
        const el = document.getElementById(id);
        if (el) {
            el.disabled = disabled;
        }
    });
}

function currentTopologyInputParentIds() {
    const pinnedParent = document.getElementById("nodeTopologyPinnedParent")?.value || "";
    return pinnedParent ? [pinnedParent] : [];
}

function topologySeedParentId(overrideData) {
    const savedIds = Array.isArray(overrideData?.override_parent_node_ids)
        ? overrideData.override_parent_node_ids.filter((id) => typeof id === "string" && id.length > 0)
        : [];
    if (savedIds.length > 0) {
        return savedIds[0];
    }
    return "";
}

function renderTopologyPinnedOptions(overrideData, selectedId) {
    const target = document.getElementById("nodeTopologyPinnedParent");
    if (!target) {
        return;
    }
    clearDiv(target);
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Default upstream parent";
    placeholder.selected = !selectedId;
    target.appendChild(placeholder);

    (overrideData?.candidate_parents || []).forEach((candidate) => {
        const option = document.createElement("option");
        option.value = candidate.node_id;
        option.textContent = candidate.node_name || candidate.node_id;
        if (selectedId === candidate.node_id) {
            option.selected = true;
        }
        target.appendChild(option);
    });
}

function maybeSeedTopologyOverrideInputs(overrideData, force = false) {
    const pinnedSelect = document.getElementById("nodeTopologyPinnedParent");
    if (!pinnedSelect) {
        return;
    }
    const candidates = Array.isArray(overrideData?.candidate_parents)
        ? overrideData.candidate_parents
        : [];
    const selectedId = topologySeedParentId(overrideData);
    const signature = JSON.stringify({
        nodeId: currentNode()?.id ?? null,
        selectedId,
        currentParentId: overrideData?.current_parent_node_id || null,
        candidates: candidates.map((candidate) => [candidate?.node_id || "", candidate?.node_name || ""]),
        hasOverride: !!overrideData?.has_override,
    });

    if (!force && nodeTopologyInputsDirty) {
        return;
    }
    if (!force && nodeTopologyLastSeedSignature === signature) {
        return;
    }

    renderTopologyPinnedOptions(overrideData, selectedId);

    nodeTopologyLastSeedSignature = signature;
    nodeTopologyInputsDirty = false;
}

function renderNodeTopologySettings(node) {
    const statusTarget = document.getElementById("nodeTopologyStatus");
    if (statusTarget) {
        if (nodeTopologyOverrideState.loading) {
            statusTarget.textContent = "Loading topology override...";
        } else if (nodeTopologyOverrideState.saving) {
            statusTarget.textContent = "Saving topology override...";
        } else if (nodeTopologyOverrideState.error) {
            statusTarget.textContent = nodeTopologyOverrideState.error;
        } else {
            statusTarget.textContent = "";
        }
    }

    renderTopologyOverrideValue(nodeTopologyOverrideState.data);
    renderAlertMessages(
        "nodeTopologyFlash",
        nodeTopologyOverrideState.flash ? [nodeTopologyOverrideState.flash.message] : [],
        nodeTopologyOverrideState.flash ? nodeTopologyOverrideState.flash.variant : "success",
    );
    renderAlertMessages(
        "nodeTopologyWarnings",
        nodeTopologyOverrideState.data?.warnings || [],
        "warning",
    );
    renderAlertMessages(
        "nodeTopologyDisabledReason",
        nodeTopologyOverrideState.data?.disabled_reason ? [nodeTopologyOverrideState.data.disabled_reason] : [],
        "secondary",
    );

    maybeSeedTopologyOverrideInputs(nodeTopologyOverrideState.data);

    const canEdit = !!nodeTopologyOverrideState.data?.can_edit
        && !nodeTopologyOverrideState.loading
        && !nodeTopologyOverrideState.saving;
    setNodeTopologyInputsDisabled(!canEdit);

    const editorSection = document.getElementById("nodeTopologyEditorSection");
    if (editorSection) {
        editorSection.hidden = !canEdit;
    }
    const clearButton = document.getElementById("nodeTopologyClear");
    if (clearButton) {
        clearButton.disabled = !canEdit || !nodeTopologyOverrideState.data?.has_override;
    }
}

function renderNodeSettings() {
    const node = currentNode();
    if (!node) {
        return;
    }
    const syntheticRoot = isSyntheticRootNode(node);

    const statusTarget = document.getElementById("nodeOverrideStatus");
    if (statusTarget) {
        if (nodeRateOverrideState.loading) {
            statusTarget.textContent = "Loading operator override...";
        } else if (nodeRateOverrideState.saving) {
            statusTarget.textContent = "Saving operator override...";
        } else if (nodeRateOverrideState.error) {
            statusTarget.textContent = nodeRateOverrideState.error;
        } else {
            statusTarget.textContent = "";
        }
    }

    const typeTarget = document.getElementById("nodeSettingsType");
    if (typeTarget) {
        typeTarget.textContent = node.type ?? "-";
    }
    const branchSizeTarget = document.getElementById("nodeSettingsBranchSize");
    if (branchSizeTarget) {
        branchSizeTarget.textContent = formatBranchSize(node);
    }
    const baseConfiguredTarget = document.getElementById("nodeSettingsBaseConfigured");
    if (baseConfiguredTarget) {
        const configured = configuredMax(node);
        baseConfiguredTarget.textContent = formatRatePair(configured[0], configured[1]);
    }
    const effectiveTarget = document.getElementById("nodeSettingsEffectiveNow");
    if (effectiveTarget) {
        const effective = effectiveMax(node);
        effectiveTarget.textContent = formatRatePair(effective[0], effective[1]);
    }

    renderOverrideValue(node, nodeRateOverrideState.data);

    renderAlertMessages(
        "nodeOverrideFlash",
        nodeRateOverrideState.flash ? [nodeRateOverrideState.flash.message] : [],
        nodeRateOverrideState.flash ? nodeRateOverrideState.flash.variant : "success",
    );
    renderCompatibilityWarningsIndicator(nodeRateOverrideState.data?.legacy_warnings || []);
    const disabledReason = syntheticRoot ? null : nodeRateOverrideState.data?.disabled_reason;
    renderAlertMessages(
        "nodeOverrideDisabledReason",
        disabledReason ? [disabledReason] : [],
        "secondary",
    );

    maybeSeedOverrideInputs(node, nodeRateOverrideState.data);

    const canEdit = !!nodeRateOverrideState.data?.can_edit && !nodeRateOverrideState.loading && !nodeRateOverrideState.saving;
    setNodeOverrideInputsDisabled(!canEdit);
    const editorSection = document.getElementById("nodeOverrideEditorSection");
    if (editorSection) {
        editorSection.hidden = !canEdit;
    }
    const clearButton = document.getElementById("nodeOverrideClear");
    if (clearButton) {
        clearButton.disabled = !canEdit || !nodeRateOverrideState.data?.has_override;
    }

    renderNodeTopologySettings(node);

    const detailsPanel = document.querySelector(".lqos-tree-details-panel");
    if (detailsPanel) {
        enableTooltipsWithin(detailsPanel);
    }
}

function currentTreeAttachedCircuitsQuery() {
    const node = currentNode();
    if (!node) {
        return null;
    }
    const query = {
        page: 0,
        page_size: 100,
        sort: "CircuitName",
        descending: false,
    };
    const nodeId = selectionLocator.nodeId || node.id;
    const nodePath = selectionLocator.nodePath || getNodePathNames(parent, node);
    if (nodeId) {
        query.node_id = nodeId;
    }
    if (nodePath) {
        query.node_path = nodePath;
    }
    return query;
}

function requestTreeAttachedCircuitsWatch(force = false) {
    const query = currentTreeAttachedCircuitsQuery();
    if (!query) {
        return;
    }
    const signature = JSON.stringify(query);
    if (!force && attachedCircuitsWatchSignature === signature) {
        return;
    }
    attachedCircuitsWatchSignature = signature;
    sendPrivateRequest({
        WatchTreeAttachedCircuits: {
            query,
        },
    });
}

async function loadNodeRateOverrideState() {
    const query = currentNodeRateQuery();
    if (!query) {
        return;
    }
    nodeRateOverrideState.loading = true;
    nodeRateOverrideState.error = null;
    nodeRateOverrideState.data = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("GetNodeRateOverride", {
            GetNodeRateOverride: {query},
        });
        nodeRateOverrideState.data = response.data || null;
        nodeRateOverrideState.error = null;
    } catch (errorMsg) {
        nodeRateOverrideState.data = null;
        nodeRateOverrideState.error = errorMsg?.message || "Unable to load override state";
    } finally {
        nodeRateOverrideState.loading = false;
        renderNodeSettings();
    }
}

async function loadNodeTopologyOverrideState() {
    const query = currentNodeTopologyQuery();
    if (!query) {
        return;
    }
    nodeTopologyOverrideState.loading = true;
    nodeTopologyOverrideState.error = null;
    nodeTopologyOverrideState.data = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("GetNodeTopologyOverride", {
            GetNodeTopologyOverride: {query},
        });
        nodeTopologyOverrideState.data = response.data || null;
        nodeTopologyOverrideState.error = null;
    } catch (errorMsg) {
        nodeTopologyOverrideState.data = null;
        nodeTopologyOverrideState.error = errorMsg?.message || "Unable to load topology override state";
    } finally {
        nodeTopologyOverrideState.loading = false;
        renderNodeSettings();
    }
}

async function saveNodeRateOverride() {
    const node = currentNode();
    if (!node || !nodeRateOverrideState.data?.can_edit || nodeRateOverrideState.saving) {
        return;
    }

    const downloadRaw = document.getElementById("nodeOverrideDownload")?.value ?? "";
    const uploadRaw = document.getElementById("nodeOverrideUpload")?.value ?? "";
    const download = downloadRaw === "" ? null : Number.parseFloat(downloadRaw);
    const upload = uploadRaw === "" ? null : Number.parseFloat(uploadRaw);
    if ((download !== null && (!Number.isFinite(download) || download < 0))
        || (upload !== null && (!Number.isFinite(upload) || upload < 0))
        || (download === null && upload === null)) {
        setNodeOverrideFlash("Enter valid non-negative download and upload rates before saving.", "danger");
        return;
    }

    nodeRateOverrideState.saving = true;
    nodeRateOverrideState.error = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("SetNodeRateOverrideResult", {
            SetNodeRateOverride: {
                update: {
                    node_id: node.id,
                    node_name: node.name,
                    download_bandwidth_mbps: download,
                    upload_bandwidth_mbps: upload,
                },
            },
        });
        if (!response.ok) {
            setNodeOverrideFlash(response.message || "Unable to save override.", "danger");
            nodeRateOverrideState.data = response.data || nodeRateOverrideState.data;
        } else {
            nodeRateOverrideState.data = response.data || nodeRateOverrideState.data;
            setNodeOverrideFlash(response.message || "Override saved.", "success");
            nodeOverrideInputsDirty = false;
            nodeOverrideLastSeedSignature = null;
            maybeSeedOverrideInputs(node, nodeRateOverrideState.data, true);
        }
    } catch (errorMsg) {
        setNodeOverrideFlash(errorMsg?.message || "Unable to save override.", "danger");
    } finally {
        nodeRateOverrideState.saving = false;
        renderNodeSettings();
    }
}

async function clearNodeRateOverride() {
    const node = currentNode();
    if (!node || !nodeRateOverrideState.data?.can_edit || nodeRateOverrideState.saving) {
        return;
    }

    nodeRateOverrideState.saving = true;
    nodeRateOverrideState.error = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("ClearNodeRateOverrideResult", {
            ClearNodeRateOverride: {
                query: currentNodeRateQuery(),
            },
        });
        if (!response.ok) {
            setNodeOverrideFlash(response.message || "Unable to clear override.", "danger");
            nodeRateOverrideState.data = response.data || nodeRateOverrideState.data;
        } else {
            nodeRateOverrideState.data = response.data || nodeRateOverrideState.data;
            nodeOverrideInputsDirty = false;
            nodeOverrideLastSeedSignature = null;
            maybeSeedOverrideInputs(node, nodeRateOverrideState.data, true);
            setNodeOverrideFlash(response.message || "Override cleared.", "success");
        }
    } catch (errorMsg) {
        setNodeOverrideFlash(errorMsg?.message || "Unable to clear override.", "danger");
    } finally {
        nodeRateOverrideState.saving = false;
        renderNodeSettings();
    }
}

async function saveNodeTopologyOverride() {
    const node = currentNode();
    if (!node || !nodeTopologyOverrideState.data?.can_edit || nodeTopologyOverrideState.saving) {
        return;
    }

    const parentNodeIds = currentTopologyInputParentIds();
    if (parentNodeIds.length === 0) {
        if (nodeTopologyOverrideState.data?.has_override) {
            await clearNodeTopologyOverride();
            return;
        }
        setNodeTopologyOverrideFlash("Default upstream parent selected. No topology override is set.", "secondary");
        return;
    }
    if (parentNodeIds.length !== 1) {
        setNodeTopologyOverrideFlash("Select exactly one pinned parent before saving.", "danger");
        return;
    }

    nodeTopologyOverrideState.saving = true;
    nodeTopologyOverrideState.error = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("SetNodeTopologyOverrideResult", {
            SetNodeTopologyOverride: {
                update: {
                    node_id: node.id,
                    node_name: node.name,
                    mode: "pinned",
                    parent_node_ids: parentNodeIds,
                },
            },
        });
        if (!response.ok) {
            setNodeTopologyOverrideFlash(response.message || "Unable to save topology override.", "danger");
            nodeTopologyOverrideState.data = response.data || nodeTopologyOverrideState.data;
        } else {
            nodeTopologyOverrideState.data = response.data || nodeTopologyOverrideState.data;
            nodeTopologyInputsDirty = false;
            nodeTopologyLastSeedSignature = null;
            maybeSeedTopologyOverrideInputs(nodeTopologyOverrideState.data, true);
            setNodeTopologyOverrideFlash(
                response.message || `${topologyOverrideLabel()} saved.`,
                "success",
            );
        }
    } catch (errorMsg) {
        setNodeTopologyOverrideFlash(errorMsg?.message || "Unable to save topology override.", "danger");
    } finally {
        nodeTopologyOverrideState.saving = false;
        renderNodeSettings();
    }
}

async function clearNodeTopologyOverride() {
    const node = currentNode();
    if (!node || !nodeTopologyOverrideState.data?.can_edit || nodeTopologyOverrideState.saving) {
        return;
    }

    nodeTopologyOverrideState.saving = true;
    nodeTopologyOverrideState.error = null;
    renderNodeSettings();
    try {
        const response = await sendWsRequest("ClearNodeTopologyOverrideResult", {
            ClearNodeTopologyOverride: {
                query: currentNodeTopologyQuery(),
            },
        });
        if (!response.ok) {
            setNodeTopologyOverrideFlash(response.message || "Unable to clear topology override.", "danger");
            nodeTopologyOverrideState.data = response.data || nodeTopologyOverrideState.data;
        } else {
            nodeTopologyOverrideState.data = response.data || nodeTopologyOverrideState.data;
            nodeTopologyInputsDirty = false;
            nodeTopologyLastSeedSignature = null;
            maybeSeedTopologyOverrideInputs(nodeTopologyOverrideState.data, true);
            setNodeTopologyOverrideFlash(response.message || "Topology override cleared.", "success");
        }
    } catch (errorMsg) {
        setNodeTopologyOverrideFlash(errorMsg?.message || "Unable to clear topology override.", "danger");
    } finally {
        nodeTopologyOverrideState.saving = false;
        renderNodeSettings();
    }
}

function formatQooScore(score0to100, fallback = "-") {
    if (score0to100 === null || score0to100 === undefined) {
        return fallback;
    }
    const numeric = Number(score0to100);
    if (!Number.isFinite(numeric) || numeric === 255) {
        return fallback;
    }
    const clamped = Math.min(100, Math.max(0, Math.round(numeric)));
    const color = colorByQoqScore(clamped);
    return "<span class='muted' style='color: " + color + "'>■</span>" + clamped;
}

function representativeNodeQoo(node) {
    if (!node || !node.qoo) {
        return null;
    }
    const values = [node.qoo[0], node.qoo[1]]
        .map((entry) => Number(entry))
        .filter((entry) => Number.isFinite(entry) && entry !== 255);
    if (values.length === 0) {
        return null;
    }
    return values.reduce((sum, entry) => sum + entry, 0) / values.length;
}

function ensureTreeGauges() {
    if (!treeBitsGauge) {
        treeBitsGauge = new BitsPerSecondGauge("treeBitsGauge", "Max");
    }
    if (!treeQooGauge) {
        treeQooGauge = new QooScoreGauge("treeQooGauge");
    }
}

function updateTreeGauges(node) {
    if (!node) {
        return;
    }
    ensureTreeGauges();
    const gaugeMax = nodeSnapshotGaugeMax(node);
    treeBitsGauge.update(
        toNumber(node.current_throughput?.[0], 0) * 8,
        toNumber(node.current_throughput?.[1], 0) * 8,
        gaugeMax[0],
        gaugeMax[1],
    );
    treeQooGauge.update(representativeNodeQoo(node));
}

function buildChildrenMap() {
    childrenByParentId = new Map();
    for (let i=0; i<tree.length; i++) {
        if (!tree[i] || !tree[i][1]) {
            continue;
        }
        let node = tree[i][1];
        if (node.immediate_parent !== null) {
            if (!childrenByParentId.has(node.immediate_parent)) {
                childrenByParentId.set(node.immediate_parent, []);
            }
            childrenByParentId.get(node.immediate_parent).push(i);
        }
    }
}

function hasChildren(nodeId) {
    let children = childrenByParentId.get(nodeId);
    return children !== undefined && children.length > 0;
}

function toggleNode(nodeId) {
    if (!hasChildren(nodeId)) {
        return;
    }
    if (expandedNodes.has(nodeId)) {
        expandedNodes.delete(nodeId);
    } else {
        expandedNodes.add(nodeId);
    }
    renderTree();
}

function renderTree() {
    const treeStack = document.createElement("div");
    treeStack.classList.add("lqos-tree-stack");
    const tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");
    let treeTable = document.createElement("table");
    treeTable.classList.add("lqos-table", "lqos-table-tight");
    let thead = document.createElement("thead");
    thead.appendChild(theading("Node"));
    thead.appendChild(theading("Circuits"));
    thead.appendChild(theading("Nodes"));
    thead.appendChild(theading("Effective"));
    thead.appendChild(theading("Configured"));
    thead.appendChild(theading("⬇️"));
    thead.appendChild(theading("⬆️"));
    thead.appendChild(theading("RTT", 2, "<h5>TCP Round-Trip Time</h5><p>Current median TCP round-trip time. Time taken for a full send-acknowledge round trip. Low numbers generally equate to a smoother user experience.</p>", "tts_retransmits"));
    thead.appendChild(theading("QoO", 2, QOO_TOOLTIP_HTML, "tts_qoo"));
    thead.appendChild(theading("Retr", 2, "<h5>TCP Retransmits</h5><p>Number of TCP retransmits in the last second.</p>", "tts_retransmits"));
    thead.appendChild(theading("Marks", 2, "<h5>Cake Marks</h5><p>Number of times the Cake traffic manager has applied ECN marks to avoid congestion.</p>", "tts_marks"));
    thead.appendChild(theading("Drops", 2, "<h5>Cake Drops</h5><p>Number of times the Cake traffic manager has dropped packets to avoid congestion.</p>", "tts_drops"));

    treeTable.appendChild(thead);
    let tbody = document.createElement("tbody");

    let topChildren = childrenByParentId.get(parent) || [];
    topChildren.forEach((childIdx) => {
        let row = buildRow(childIdx);
        tbody.appendChild(row);
        let childId = tree[childIdx][0];
        if (expandedNodes.has(childId)) {
            iterateChildren(childIdx, tbody, 1);
        }
    });

    treeTable.appendChild(tbody);

    // Clear and apply
    let target = document.getElementById("tree");
    clearDiv(target)
    tableWrap.appendChild(treeTable);
    treeStack.appendChild(tableWrap);
    target.appendChild(treeStack);
    enableTooltipsWithin(treeTable);
}

// This runs first and builds the initial structure on the page
function getInitialTree() {
    listenOnce("NetworkTree", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        tree = data;
        buildChildrenMap();
        reconcileSelection();
        if (tree[parent] !== undefined) {
            fillHeader(tree[parent][1]);
            loadNodeRateOverrideState();
            loadNodeTopologyOverrideState();
        }
        renderTree();
        requestTreeAttachedCircuitsWatch(true);

        if (!subscribed) {
            subscribeWS(["NetworkTree", "StormguardStatus"], onMessage);
            subscribed = true;
        }
    });
    wsClient.send({ NetworkTree: {} });
}

function fillHeader(node) {
    renderHeaderStatusIcons(node);
    renderBreadcrumb();
    renderContextMeta(node);
    updateTreeGauges(node);
    const configured = configuredMax(node);
    const effective = effectiveMax(node);
    const configuredDown = formatLimitValue(configured[0]);
    const configuredUp = formatLimitValue(configured[1]);
    const effectiveDown = formatLimitValue(effective[0]);
    const effectiveUp = formatLimitValue(effective[1]);
    const matchClassDown = Math.abs(toNumber(effective[0], 0) - toNumber(configured[0], 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS
        ? "lqos-limit-secondary is-match"
        : "lqos-limit-secondary";
    const matchClassUp = Math.abs(toNumber(effective[1], 0) - toNumber(configured[1], 0)) <= THROUGHPUT_COMPARE_EPSILON_MBPS
        ? "lqos-limit-secondary is-match"
        : "lqos-limit-secondary";
    $("#parentEffectiveD").text(effectiveDown);
    $("#parentEffectiveU").text(effectiveUp);
    $("#parentConfiguredD").text(configuredDown).removeClass("lqos-limit-secondary is-match").addClass(matchClassDown);
    $("#parentConfiguredU").text(configuredUp).removeClass("lqos-limit-secondary is-match").addClass(matchClassUp);
    $("#parentTpD").html(formatThroughput(toNumber(node.current_throughput[0], 0) * 8, effective[0]));
    $("#parentTpU").html(formatThroughput(toNumber(node.current_throughput[1], 0) * 8, effective[1]));
    $("#parentRttD").html(formatRtt(node.rtts[0]));
    $("#parentRttU").html(formatRtt(node.rtts[1]));
    $("#parentQooD").html(formatQooScore(node.qoo ? node.qoo[0] : null));
    $("#parentQooU").html(formatQooScore(node.qoo ? node.qoo[1] : null));
    let retr = 0;
    const packetsDown = retransmitPacketsForNode(node, 0);
    if (packetsDown > 0) {
        retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
    }
    $("#parentRxmitD").html(formatRetransmit(retr));
    retr = 0;
    const packetsUp = retransmitPacketsForNode(node, 1);
    if (packetsUp > 0) {
        retr = toNumber(node.current_retransmits[1], 0) / packetsUp;
    }
    $("#parentRxmitU").html(formatRetransmit(retr));
    renderNodeSettings();
}

function iterateChildren(idx, tBody, depth) {
    let nodeId = tree[idx][0];
    let children = childrenByParentId.get(nodeId) || [];
    children.forEach((childIdx) => {
        let row = buildRow(childIdx, depth);
        tBody.appendChild(row);
        let childId = tree[childIdx][0];
        if (expandedNodes.has(childId)) {
            iterateChildren(childIdx, tBody, depth + 1);
        }
    });
}

function buildRow(i, depth=0) {
    let node = tree[i][1];
    let nodeId = tree[i][0];
    let row = document.createElement("tr");
    row.classList.add("small");
    let col = document.createElement("td");
    col.classList.add("small", "lqos-tree-node-cell");
    let nameWrap = document.createElement("div");
    nameWrap.classList.add("lqos-tree-node-wrap");
    if (depth > 0) {
        nameWrap.style.paddingLeft = (depth * 1.25) + "rem";
    }
    const lead = document.createElement("div");
    lead.classList.add("lqos-tree-node-lead");
    const toggleSlot = document.createElement("div");
    toggleSlot.classList.add("lqos-tree-toggle-slot");
    if (hasChildren(nodeId)) {
        let toggle = document.createElement("button");
        toggle.type = "button";
        toggle.classList.add("btn", "btn-link", "btn-sm", "p-0", "text-decoration-none", "lqos-tree-toggle");
        let icon = document.createElement("i");
        icon.classList.add("fa", "fa-fw", expandedNodes.has(nodeId) ? "fa-minus" : "fa-plus");
        toggle.appendChild(icon);
        toggle.title = expandedNodes.has(nodeId) ? "Collapse" : "Expand";
        toggle.setAttribute("aria-label", toggle.title);
        toggle.addEventListener("click", (event) => {
            event.preventDefault();
            event.stopPropagation();
            toggleNode(nodeId);
        });
        toggleSlot.appendChild(toggle);
    }
    lead.appendChild(toggleSlot);
    let link = document.createElement("a");
    link.href = treeHref(nodeId, node);
    link.classList.add("redactable", "lqos-tree-node-link");
    link.textContent = node.name;
    lead.appendChild(link);
    nameWrap.appendChild(lead);
    const statusWrap = document.createElement("div");
    statusWrap.classList.add("lqos-tree-row-statuses");
    statusWrap.appendChild(buildTreeRowStatusIcon(
        "fa-ghost",
        node.virtual === true,
        "Virtual node: Active",
        "Virtual node: Inactive",
    ));
    statusWrap.appendChild(buildTreeRowStatusIcon(
        "fa-layer-group",
        node.runtime_virtualized === true,
        "TreeGuard runtime virtualization: Active",
        "TreeGuard runtime virtualization: Inactive",
    ));
    statusWrap.appendChild(buildTreeRowStatusIcon(
        "fa-cloud-bolt",
        isStormguardNode(node),
        "StormGuard: Active",
        "StormGuard: Inactive",
    ));
    nameWrap.appendChild(statusWrap);
    col.appendChild(nameWrap);
    row.appendChild(col);

    col = document.createElement("td");
    col.classList.add("small");
    col.textContent = formatTreeCount(subtreeCircuitCount(node));
    row.appendChild(col);

    col = document.createElement("td");
    col.classList.add("small");
    col.textContent = formatTreeCount(subtreeSiteCount(node));
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "limit-" + nodeId;
    col.classList.add("small");
    col.style.width = "8%";
    col.innerHTML = buildEffectiveLimitCellHtml(node);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "configured-limit-" + nodeId;
    col.classList.add("small");
    col.style.width = "8%";
    col.innerHTML = buildConfiguredLimitCellHtml(node);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "down-" + nodeId;
    col.classList.add("small");
    col.style.width = "5%";
    col.innerHTML = formatThroughput(toNumber(node.current_throughput[0], 0) * 8, effectiveMax(node)[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "up-" + nodeId;
    col.classList.add("small");
    col.style.width = "5%";
    col.innerHTML = formatThroughput(toNumber(node.current_throughput[1], 0) * 8, effectiveMax(node)[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-down-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatRtt(node.rtts[0]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "rtt-up-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatRtt(node.rtts[1]);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "qoo-down-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatQooScore(node.qoo ? node.qoo[0] : null);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "qoo-up-" + nodeId;
    col.style.width = "6%";
    col.innerHTML = formatQooScore(node.qoo ? node.qoo[1] : null);
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_retransmits[0] !== undefined) {
        let retr = 0;
        const packetsDown = retransmitPacketsForNode(node, 0);
        if (packetsDown > 0) {
            retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
        }
        col.innerHTML = formatRetransmitFraction(retr);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-up-" + nodeId;
    col.style.width = "6%";
    if (node.current_retransmits[1] !== undefined) {
        let retr = 0;
        const packetsUp = retransmitPacketsForNode(node, 1);
        if (packetsUp > 0) {
            retr = toNumber(node.current_retransmits[1], 0) / packetsUp;
        }
        col.innerHTML = formatRetransmitFraction(retr);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_marks[0] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_marks[0], node.current_packets[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "ecn-up-" + nodeId;
    col.style.width = "6%";
    if (node.current_marks[1] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_marks[1], node.current_packets[1]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-down-" + nodeId;
    col.style.width = "6%";
    if (node.current_drops[0] !== undefined) {
        col.innerHTML = formatCakeStatPercent(node.current_drops[0], node.current_packets[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "drops-up-" + nodeId;
    //col.style.width = "6%";
    if (node.current_drops[1] !== undefined) {
        col.innerHTML = formatCakeStat(node.current_drops[1], node.current_packets[1]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    return row;
}

function treeUpdate(msg) {
    //console.log(msg);
    let needsRebuild = false;
    const seenNodeIds = new Set();
    msg.data.forEach((n) => {
        let nodeId = n[0];
        let node = n[1];
        seenNodeIds.add(nodeId);

        if (tree[nodeId] === undefined) {
            tree[nodeId] = [nodeId, node];
            needsRebuild = true;
        } else {
            if (tree[nodeId][1].immediate_parent !== node.immediate_parent
                || tree[nodeId][1].subtree_site_count !== node.subtree_site_count
                || tree[nodeId][1].subtree_circuit_count !== node.subtree_circuit_count
                || tree[nodeId][1].type !== node.type
                || tree[nodeId][1].virtual !== node.virtual
                || tree[nodeId][1].runtime_virtualized !== node.runtime_virtualized
                || tree[nodeId][1].name !== node.name) {
                needsRebuild = true;
            }
            tree[nodeId][1] = node;
        }

        let col = document.getElementById("limit-" + nodeId);
        if (col !== null) {
            col.innerHTML = buildEffectiveLimitCellHtml(node);
        }
        col = document.getElementById("configured-limit-" + nodeId);
        if (col !== null) {
            col.innerHTML = buildConfiguredLimitCellHtml(node);
        }

        col = document.getElementById("down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(toNumber(node.current_throughput[0], 0) * 8, effectiveMax(node)[0]);
        }
        col = document.getElementById("up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatThroughput(toNumber(node.current_throughput[1], 0) * 8, effectiveMax(node)[1]);
        }
        col = document.getElementById("rtt-down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[0]);
        }
        col = document.getElementById("rtt-up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatRtt(node.rtts[1]);
        }
        col = document.getElementById("qoo-down-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatQooScore(node.qoo ? node.qoo[0] : null);
        }
        col = document.getElementById("qoo-up-" + nodeId);
        if (col !== null) {
            col.innerHTML = formatQooScore(node.qoo ? node.qoo[1] : null);
        }
        col = document.getElementById("re-xmit-down-" + nodeId);
        if (col !== null) {
            if (node.current_retransmits[0] !== undefined) {
                let retr = 0;
                const packetsDown = retransmitPacketsForNode(node, 0);
                if (packetsDown > 0) {
                    retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
                }
                col.innerHTML = formatRetransmit(retr);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("re-xmit-up-" + nodeId);
        if (col !== null) {
            if (node.current_retransmits[1] !== undefined) {
                let retr = 0;
                const packetsUp = retransmitPacketsForNode(node, 1);
                if (packetsUp > 0) {
                    retr = toNumber(node.current_retransmits[1], 0) / packetsUp;
                }
                col.innerHTML = formatRetransmit(retr);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("ecn-down-" + nodeId);
        if (col !== null) {
            if (node.current_marks[0] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_marks[0], node.current_packets[0]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("ecn-up-" + nodeId);
        if (col !== null) {
            if (node.current_marks[1] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_marks[1], node.current_packets[1]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("drops-down-" + nodeId);
        if (col !== null) {
            if (node.current_drops[0] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_drops[0], node.current_packets[0]);
            } else {
                col.textContent = "-";
            }
        }
        col = document.getElementById("drops-up-" + nodeId);
        if (col !== null) {
            if (node.current_drops[1] !== undefined) {
                col.innerHTML = formatCakeStatPercent(node.current_drops[1], node.current_packets[1]);
            } else {
                col.textContent = "-";
            }
        }
    });
    for (let i = 0; i < tree.length; i++) {
        if (tree[i] === undefined || seenNodeIds.has(i)) {
            continue;
        }
        delete tree[i];
        needsRebuild = true;
    }
    if (needsRebuild) {
        buildChildrenMap();
    }
    const selectionState = reconcileSelection();
    if (selectionState.identityChanged) {
        loadNodeRateOverrideState();
        loadNodeTopologyOverrideState();
    }
    if (tree[parent] !== undefined) {
        fillHeader(tree[parent][1]);
    }
    if (needsRebuild || selectionState.parentChanged) {
        renderTree();
    }
    if (selectionState.parentChanged || selectionState.identityChanged) {
        requestTreeAttachedCircuitsWatch(true);
    }
}

function renderAttachedCircuitsRows(rows) {
    if (!Array.isArray(rows)) {
        return;
    }

    let target = document.getElementById("clients");
    let table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight");
    table.appendChild(clientTableHeader());
    let tbody = document.createElement("tbody");
    clearDiv(target);
    rows.forEach((circuit) => {
            let tr = document.createElement("tr");
            tr.classList.add("small");

            let linkTd = document.createElement("td");
            let circuitLink = document.createElement("a");
            circuitLink.href = "/circuit.html?id=" + circuit.circuit_id;
            circuitLink.innerText = circuit.circuit_name;
            circuitLink.classList.add("redactable");
            circuitLink.classList.add("lqos-tree-circuit-name");
            circuitLink.title = circuit.circuit_name;
            linkTd.appendChild(circuitLink);
            tr.appendChild(linkTd);

            const deviceNames = Array.isArray(circuit.device_names) ? circuit.device_names : [];
            const deviceCell = simpleRow(
                deviceNames.length > 2 ? `${deviceNames[0]}, ${deviceNames[1]} +${deviceNames.length - 2}` : deviceNames.join(", "),
                true
            );
            if (deviceNames.length > 0) {
                deviceCell.title = deviceNames.join(", ");
            }
            tr.appendChild(deviceCell);

            const planCell = simpleRow(
                `${toNumber(circuit.plan_mbps?.down, 0)} / ${toNumber(circuit.plan_mbps?.up, 0)}`
            );
            planCell.classList.add("lqos-tree-plan-cell");
            if (circuit.ethernet_cap_badge) {
                const badge = document.createElement("a");
                badge.className = `badge rounded-pill ms-2 text-decoration-none ${ethernetBadgeClass(circuit.ethernet_cap_badge.tier_label)}`;
                badge.href = ethernetCapsPageHref(circuit.ethernet_cap_badge);
                badge.setAttribute("aria-label", `Review ${circuit.ethernet_cap_badge.tier_label} Ethernet-limited circuits`);
                badge.setAttribute("data-bs-toggle", "tooltip");
                badge.setAttribute("data-bs-placement", "top");
                badge.setAttribute("title", formatEthernetTooltip(circuit.ethernet_cap_badge));
                badge.textContent = circuit.ethernet_cap_badge.tier_label;
                planCell.appendChild(badge);
            }
            tr.appendChild(planCell);
            tr.appendChild(simpleRow(circuit.parent_node, true));

            const ipList = Array.isArray(circuit.ip_addrs) ? circuit.ip_addrs : [];
            const ipCell = simpleRow(summarizeIpListForTable(ipList), true);
            ipCell.classList.add("lqos-tree-ip-cell");
            if (ipList.length > 0) {
                ipCell.title = ipList.join(", ");
            }
            tr.appendChild(ipCell);

            tr.appendChild(simpleRow(formatLastSeen(toNumber(circuit.last_seen_nanos, 0))));
            tr.appendChild(simpleRowHtml(formatThroughput(toNumber(circuit.bytes_per_second?.down, 0) * 8, toNumber(circuit.plan_mbps?.down, 0))));
            tr.appendChild(simpleRowHtml(formatThroughput(toNumber(circuit.bytes_per_second?.up, 0) * 8, toNumber(circuit.plan_mbps?.up, 0))));

            if (toNumber(circuit.rtt_current_p50_nanos?.down, 0) > 0) {
                tr.appendChild(simpleRowHtml(formatRtt(toNumber(circuit.rtt_current_p50_nanos?.down, 0) / 1_000_000)));
            } else {
                tr.appendChild(simpleRow("-"));
            }
            if (toNumber(circuit.rtt_current_p50_nanos?.up, 0) > 0) {
                tr.appendChild(simpleRowHtml(formatRtt(toNumber(circuit.rtt_current_p50_nanos?.up, 0) / 1_000_000)));
            } else {
                tr.appendChild(simpleRow("-"));
            }

            tr.appendChild(simpleRowHtml(formatRetransmit(retransmitFractionFromSample(circuit.tcp_retransmit_sample?.down))));
            tr.appendChild(simpleRowHtml(formatRetransmit(retransmitFractionFromSample(circuit.tcp_retransmit_sample?.up))));

            tbody.appendChild(tr);
    });
    table.appendChild(tbody);
    const sectionLabel = document.createElement("div");
    sectionLabel.classList.add("lqos-tree-section-label");
    sectionLabel.innerHTML = "<i class='fa fa-network-wired'></i> Attached Circuits";
    const tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");
    tableWrap.appendChild(table);
    target.appendChild(sectionLabel);
    target.appendChild(tableWrap);
    enableTooltipsWithin(target);
}

function attachedCircuitsUpdate(msg) {
    lastAttachedCircuitsPage = msg?.data || null;
    renderAttachedCircuitsRows(lastAttachedCircuitsPage?.rows || []);
}

function stormguardUpdate(msg) {
    const nextNodes = new Set();
    msg.data.forEach((entry) => {
        if (!Array.isArray(entry) || typeof entry[0] !== "string" || entry[0].length === 0) {
            return;
        }
        nextNodes.add(entry[0]);
    });
    if (sameStringSet(stormguardNodes, nextNodes)) {
        return;
    }
    stormguardNodes = nextNodes;
    const selectionState = reconcileSelection();
    if (selectionState.parentChanged || selectionState.identityChanged) {
        requestTreeAttachedCircuitsWatch(true);
    }
    if (tree && tree[parent] !== undefined) {
        fillHeader(tree[parent][1]);
        renderTree();
    }
}

function onMessage(msg) {
    if (msg.event === "NetworkTree") {
        treeUpdate(msg);
    } else if (msg.event === "StormguardStatus") {
        stormguardUpdate(msg);
    }
}

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

if (params.parent !== null) {
    const parsedParent = parseInt(params.parent, 10);
    parent = Number.isFinite(parsedParent) ? parsedParent : 0;
} else {
    parent = 0;
}

if (params.upParent !== null) {
    const parsedUpParent = parseInt(params.upParent, 10);
    upParent = Number.isFinite(parsedUpParent) ? parsedUpParent : 0;
}

selectionLocator.nodeId = typeof params.nodeId === "string" && params.nodeId.length > 0 ? params.nodeId : null;
selectionLocator.nodePath = parseNodePathParam(params.nodePath);
selectionLocator.lastKnownIndex = parent;

document.getElementById("nodeOverrideDownload")?.addEventListener("input", () => {
    nodeOverrideInputsDirty = true;
});
document.getElementById("nodeOverrideUpload")?.addEventListener("input", () => {
    nodeOverrideInputsDirty = true;
});
document.getElementById("nodeOverrideSave")?.addEventListener("click", () => {
    saveNodeRateOverride();
});
document.getElementById("nodeOverrideClear")?.addEventListener("click", () => {
    clearNodeRateOverride();
});
document.getElementById("nodeTopologyPinnedParent")?.addEventListener("change", () => {
    nodeTopologyInputsDirty = true;
});
document.getElementById("nodeTopologySave")?.addEventListener("click", () => {
    saveNodeTopologyOverride();
});
document.getElementById("nodeTopologyClear")?.addEventListener("click", () => {
    clearNodeTopologyOverride();
});
wsClient.on("TreeAttachedCircuitsSnapshot", attachedCircuitsUpdate);
wsClient.on("TreeAttachedCircuitsUpdate", attachedCircuitsUpdate);
wsClient.on("join", () => {
    if (!tree) {
        return;
    }
    requestTreeAttachedCircuitsWatch(true);
    loadNodeRateOverrideState();
    loadNodeTopologyOverrideState();
});

loadRootGaugeConfigMax();
getInitialTree();
