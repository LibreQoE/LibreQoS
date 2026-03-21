import {clearDiv, clientTableHeader, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {
    formatCakeStat, formatCakeStatPercent,
    formatRetransmit, formatRetransmitRaw,
    formatRtt,
    formatThroughput,
} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
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
var nodeRateOverrideState = {
    loading: false,
    saving: false,
    data: null,
    error: null,
    flash: null,
};
var nodeOverrideInputsDirty = false;
var nodeOverrideLastSeedSignature = null;
const wsClient = get_ws_client();
const QOO_TOOLTIP_HTML = "<h5>Quality of Outcome (QoO)</h5>" +
    "<p>Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).<br>" +
    "https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/<br>" +
    "LibreQoS implements a latency and loss-based model to estimate quality of outcome.</p>";
const THROUGHPUT_COMPARE_EPSILON_MBPS = 0.01;
const NODE_OVERRIDE_PENDING_TOOLTIP = "Stored as an operator override. Will be applied to generated network.json on the next scheduler run.";

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

function configuredMax(node) {
    return node.configured_max_throughput || node.max_throughput || [0, 0];
}

function effectiveMax(node) {
    return node.effective_max_throughput || configuredMax(node);
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

function currentNode() {
    return tree && tree[parent] ? tree[parent][1] : null;
}

function currentNodeRateQuery() {
    const node = currentNode();
    if (!node) {
        return null;
    }
    return {
        node_id: node.id ?? null,
        node_name: node.name,
    };
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

function nodeDetailParts(node) {
    const parts = [];
    if (node.type !== null && node.type !== undefined && node.type !== "") {
        parts.push(node.type);
    }
    const circuitCount = subtreeCircuitCount(node);
    if (circuitCount > 0) {
        parts.push(`Circuits ${circuitCount}`);
    }
    const siteCount = subtreeSiteCount(node);
    if (siteCount > 0) {
        parts.push(`Sites ${siteCount}`);
    }
    return parts;
}

function appendNodeDetailText(target, node) {
    const parts = nodeDetailParts(node);
    if (parts.length === 0) {
        return;
    }
    const detail = document.createElement("span");
    detail.classList.add("text-body-secondary");
    detail.textContent = ` (${parts.join(", ")})`;
    target.appendChild(detail);
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
    const target = document.getElementById("nodeNameIcons");
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

function treeHref(nodeId) {
    return `/tree.html?parent=${nodeId}`;
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
    const navWrap = document.createElement("div");
    navWrap.classList.add("lqos-tree-nav");
    const trail = document.createElement("div");
    trail.classList.add("lqos-tree-breadcrumb");

    path.forEach((nodeId, index) => {
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

        if (index === path.length - 1) {
            const currentNode = document.createElement("span");
            currentNode.classList.add("lqos-tree-breadcrumb-current", "redactable");
            currentNode.textContent = entry[1].name;
            trail.appendChild(currentNode);
        } else {
            const link = document.createElement("a");
            link.href = treeHref(nodeId);
            link.classList.add("lqos-tree-breadcrumb-link", "redactable");
            link.textContent = entry[1].name;
            trail.appendChild(link);
        }
    });

    navWrap.appendChild(trail);
    target.appendChild(navWrap);
}

function setNodeOverrideFlash(message, variant = "success") {
    nodeRateOverrideState.flash = message ? {message, variant} : null;
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
    clearDiv(target);
    const wrap = document.createElement("span");
    wrap.classList.add("lqos-tree-settings-value");

    const value = document.createElement("span");
    if (!overrideData || !overrideData.has_override) {
        value.textContent = "None";
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

function renderNodeSettings() {
    const node = currentNode();
    if (!node) {
        return;
    }

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
    const nodeIdTarget = document.getElementById("nodeSettingsNodeId");
    if (nodeIdTarget) {
        nodeIdTarget.textContent = node.id ?? "Unavailable";
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
    renderAlertMessages(
        "nodeOverrideLegacyWarnings",
        nodeRateOverrideState.data?.legacy_warnings || [],
        "warning",
    );
    renderAlertMessages(
        "nodeOverrideDisabledReason",
        nodeRateOverrideState.data?.disabled_reason ? [nodeRateOverrideState.data.disabled_reason] : [],
        "secondary",
    );

    maybeSeedOverrideInputs(node, nodeRateOverrideState.data);

    const canEdit = !!nodeRateOverrideState.data?.can_edit && !nodeRateOverrideState.loading && !nodeRateOverrideState.saving;
    setNodeOverrideInputsDisabled(!canEdit);
    const clearButton = document.getElementById("nodeOverrideClear");
    if (clearButton) {
        clearButton.disabled = !canEdit || !nodeRateOverrideState.data?.has_override;
    }

    const settingsCard = document.querySelector(".lqos-tree-settings-card");
    if (settingsCard) {
        enableTooltipsWithin(settingsCard);
    }
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
                query: {
                    node_id: node.id ?? null,
                    node_name: node.name,
                },
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

function buildChildrenMap() {
    childrenByParentId = new Map();
    for (let i=0; i<tree.length; i++) {
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
    thead.appendChild(theading("Name"));
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
        if (tree[parent] !== undefined) {
            fillHeader(tree[parent][1]);
            loadNodeRateOverrideState();
        }
        renderTree();

        if (!subscribed) {
            subscribeWS(["NetworkTree", "NetworkTreeClients", "StormguardStatus"], onMessage);
            subscribed = true;
        }
    });
    wsClient.send({ NetworkTree: {} });
}

function fillHeader(node) {
    $("#nodeName").text(node.name);
    renderHeaderStatusIcons(node);
    const summaryTarget = document.getElementById("treeHeaderSummary");
    if (summaryTarget) {
        summaryTarget.textContent = nodeDetailParts(node).join(" / ");
    }
    renderBreadcrumb();
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
    const packetsDown = toNumber(node.current_tcp_packets[0], 0);
    if (packetsDown > 0) {
        retr = toNumber(node.current_retransmits[0], 0) / packetsDown;
    }
    $("#parentRxmitD").html(formatRetransmit(retr));
    retr = 0;
    const packetsUp = toNumber(node.current_tcp_packets[1], 0);
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
    col.style.textOverflow = "ellipsis";
    col.classList.add("small");
    if (depth > 0) {
        col.style.paddingLeft = (depth * 1.25) + "rem";
    }
    let nameWrap = document.createElement("div");
    nameWrap.classList.add("d-flex", "align-items-center", "gap-1");
    if (hasChildren(nodeId)) {
        let toggle = document.createElement("button");
        toggle.type = "button";
        toggle.classList.add("btn", "btn-link", "btn-sm", "p-0", "text-decoration-none");
        toggle.style.lineHeight = "1";
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
        nameWrap.appendChild(toggle);
    } else {
        let spacer = document.createElement("i");
        spacer.classList.add("fa", "fa-fw", "fa-plus");
        spacer.style.visibility = "hidden";
        nameWrap.appendChild(spacer);
    }
    if (node.virtual === true) {
        nameWrap.appendChild(buildStatusIcon(
            "fa-ghost",
            "text-secondary",
            "Virtual node (logical only; not shaped in HTB)."
        ));
    }
    if (isStormguardNode(node)) {
        nameWrap.appendChild(buildStatusIcon(
            "fa-cloud-bolt",
            "text-primary",
            "StormGuard-managed node (dynamic queue limits active)."
        ));
    }
    let link = document.createElement("a");
    link.href = treeHref(nodeId);
    link.classList.add("redactable");
    link.textContent = node.name;
    nameWrap.appendChild(link);
    appendNodeDetailText(nameWrap, node);
    col.appendChild(nameWrap);
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
        col.innerHTML = formatRetransmitRaw(node.current_retransmits[0]);
    } else {
        col.textContent = "-";
    }
    row.appendChild(col);

    col = document.createElement("td");
    col.id = "re-xmit-up-" + nodeId;
    col.style.width = "6%";
    if (node.current_retransmits[1] !== undefined) {
        col.innerHTML = formatRetransmitRaw(node.current_retransmits[1]);
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
    msg.data.forEach((n) => {
        let nodeId = n[0];
        let node = n[1];

        if (tree[nodeId] === undefined) {
            tree[nodeId] = [nodeId, node];
            needsRebuild = true;
        } else {
            if (tree[nodeId][1].immediate_parent !== node.immediate_parent
                || tree[nodeId][1].subtree_site_count !== node.subtree_site_count
                || tree[nodeId][1].subtree_circuit_count !== node.subtree_circuit_count
                || tree[nodeId][1].type !== node.type
                || tree[nodeId][1].virtual !== node.virtual
                || tree[nodeId][1].name !== node.name) {
                needsRebuild = true;
            }
            tree[nodeId][1] = node;
        }

        if (nodeId === parent) {
            fillHeader(node);
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
                const packetsDown = toNumber(node.current_tcp_packets[0], 0);
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
                const packetsUp = toNumber(node.current_tcp_packets[1], 0);
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
    if (needsRebuild) {
        buildChildrenMap();
        renderTree();
    }
}

function clientsUpdate(msg) {
    let myName = tree[parent][1].name;

    let target = document.getElementById("clients");
    let table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight");
    table.appendChild(clientTableHeader());
    let tbody = document.createElement("tbody");
    clearDiv(target);

    const circuits = new Map();
    msg.data.forEach((device) => {
        if (device.parent_node !== myName) {
            return;
        }

        const circuitId = device.circuit_id || `${device.parent_node || ""}:${device.circuit_name || ""}`;
        if (!circuits.has(circuitId)) {
            circuits.set(circuitId, {
                circuit_id: circuitId,
                circuit_name: device.circuit_name || "(Unknown circuit)",
                parent_node: device.parent_node || "",
                plan: {
                    down: toNumber(device.plan?.down, 0),
                    up: toNumber(device.plan?.up, 0),
                },
                device_names: new Set(),
                ips: new Set(),
                last_seen_nanos: toNumber(device.last_seen_nanos, 0),
                bytes_per_second: {down: 0, up: 0},
                median_latency: {down: null, up: null},
                tcp_packets: {down: 0, up: 0},
                tcp_retransmits: {down: 0, up: 0},
            });
        }

        const circuit = circuits.get(circuitId);
        if (device.device_name) {
            circuit.device_names.add(device.device_name);
        }
        const ipText = formatDeviceIp(device.ip);
        if (ipText && ipText !== "-") {
            circuit.ips.add(ipText);
        }

        circuit.last_seen_nanos = Math.min(circuit.last_seen_nanos, toNumber(device.last_seen_nanos, 0));
        circuit.bytes_per_second.down += toNumber(device.bytes_per_second?.down, 0);
        circuit.bytes_per_second.up += toNumber(device.bytes_per_second?.up, 0);
        circuit.tcp_packets.down += toNumber(device.tcp_packets?.down, 0);
        circuit.tcp_packets.up += toNumber(device.tcp_packets?.up, 0);
        circuit.tcp_retransmits.down += toNumber(device.tcp_retransmits?.down, 0);
        circuit.tcp_retransmits.up += toNumber(device.tcp_retransmits?.up, 0);

        const downLatency = toNumber(device.median_latency?.down, 0);
        if (downLatency > 0 && (circuit.median_latency.down === null || downLatency > circuit.median_latency.down)) {
            circuit.median_latency.down = downLatency;
        }
        const upLatency = toNumber(device.median_latency?.up, 0);
        if (upLatency > 0 && (circuit.median_latency.up === null || upLatency > circuit.median_latency.up)) {
            circuit.median_latency.up = upLatency;
        }
    });

    Array.from(circuits.values())
        .sort((a, b) => a.circuit_name.localeCompare(b.circuit_name))
        .forEach((circuit) => {
            let tr = document.createElement("tr");
            tr.classList.add("small");

            let linkTd = document.createElement("td");
            let circuitLink = document.createElement("a");
            circuitLink.href = "/circuit.html?id=" + circuit.circuit_id;
            circuitLink.innerText = circuit.circuit_name;
            circuitLink.classList.add("redactable");
            linkTd.appendChild(circuitLink);
            tr.appendChild(linkTd);

            const deviceNames = Array.from(circuit.device_names);
            const deviceCell = simpleRow(
                deviceNames.length > 2 ? `${deviceNames[0]}, ${deviceNames[1]} +${deviceNames.length - 2}` : deviceNames.join(", "),
                true
            );
            if (deviceNames.length > 0) {
                deviceCell.title = deviceNames.join(", ");
            }
            tr.appendChild(deviceCell);

            tr.appendChild(simpleRow(circuit.plan.down + " / " + circuit.plan.up));
            tr.appendChild(simpleRow(circuit.parent_node, true));

            const ipList = Array.from(circuit.ips);
            const ipCell = simpleRow(
                ipList.length > 2 ? `${ipList[0]}, ${ipList[1]} +${ipList.length - 2}` : ipList.join(", "),
                true
            );
            if (ipList.length > 0) {
                ipCell.title = ipList.join(", ");
            }
            tr.appendChild(ipCell);

            tr.appendChild(simpleRow(formatLastSeen(circuit.last_seen_nanos)));
            tr.appendChild(simpleRowHtml(formatThroughput(circuit.bytes_per_second.down * 8, circuit.plan.down)));
            tr.appendChild(simpleRowHtml(formatThroughput(circuit.bytes_per_second.up * 8, circuit.plan.up)));

            if (circuit.median_latency.down !== null) {
                tr.appendChild(simpleRowHtml(formatRtt(circuit.median_latency.down)));
            } else {
                tr.appendChild(simpleRow("-"));
            }
            if (circuit.median_latency.up !== null) {
                tr.appendChild(simpleRowHtml(formatRtt(circuit.median_latency.up)));
            } else {
                tr.appendChild(simpleRow("-"));
            }

            let retr = 0;
            if (circuit.tcp_packets.down > 0) {
                retr = circuit.tcp_retransmits.down / circuit.tcp_packets.down;
            }
            tr.appendChild(simpleRowHtml(formatRetransmit(retr)));

            retr = 0;
            if (circuit.tcp_packets.up > 0) {
                retr = circuit.tcp_retransmits.up / circuit.tcp_packets.up;
            }
            tr.appendChild(simpleRowHtml(formatRetransmit(retr)));

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
    if (tree && tree[parent] !== undefined) {
        fillHeader(tree[parent][1]);
        renderTree();
    }
}

function onMessage(msg) {
    if (msg.event === "NetworkTree") {
        treeUpdate(msg);
    } else if (msg.event === "NetworkTreeClients") {
        clientsUpdate(msg);
    } else if (msg.event === "StormguardStatus") {
        stormguardUpdate(msg);
    }
}

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

if (params.parent !== null) {
    parent = parseInt(params.parent);
} else {
    parent = 0;
}

if (params.upParent !== null) {
    upParent = parseInt(params.upParent);
}

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

getInitialTree();
