import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const MAP_VIEWBOX = {width: 1200, height: 820};
const MAP_CENTER_Y = 410;
const LANE_STEP = 140;
const CHILD_LANE_STEP = 96;
const MAP_NODE_HALF_WIDTH = 78;
const MAP_NODE_HALF_HEIGHT = 36;
const MAP_ZOOM_MIN = 0.65;
const MAP_ZOOM_MAX = 2.6;
const MAP_ZOOM_STEP = 0.14;
const MAP_PAN_THRESHOLD_SQ = 25;
const ROOT_NODE_ID = "__topology_root__";
const MAX_CHILD_PREVIEW_NODES = 6;
const MAX_SEARCH_SUGGESTIONS = 8;

let topologyManagerState = null;
let topologyNodeById = new Map();
let topologyNodesSorted = [];
let topologyLabelById = new Map();

let networkTree = [];
let treeNodeById = new Map();
let treeNodeByIndex = new Map();
let treeNodeIndexById = new Map();
let childrenByNodeId = new Map();

let selectedNodeId = null;
let proposedParentId = null;
let proposedMode = "auto";
let proposedAttachmentIds = [];
let lastFlash = null;
let moveMode = false;
let attachmentEditMode = false;
let manualAttachmentEditMode = false;
let manualAttachmentDraftRows = [];
let dragActive = false;
let dragMoved = false;
let dragHoverParentId = null;
let dragStart = null;
let suppressNextMapClick = false;
let mapPanActive = false;
let mapPanMoved = false;
let mapPanStart = null;
let mapPanStartView = null;
let mapView = {scale: 1, x: 0, y: 0};
let searchSuggestionNodeIds = [];
let searchSuggestionIndex = -1;

function listenOnce(eventName, handler) {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
}

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

function escapeHtml(text) {
    if (text === null || text === undefined) {
        return "";
    }
    return String(text)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;")
        .replaceAll("'", "&#39;");
}

function truncateLabel(text, max = 20) {
    if (!text) return "";
    return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function nodeKindLabel(nodeId) {
    if (!nodeId) return "Node";
    if (nodeId === ROOT_NODE_ID) return "Root";
    if (nodeId.startsWith("uisp:site:")) return "Site";
    if (nodeId.startsWith("uisp:device:")) return "Device";
    if (nodeId.startsWith("libreqos:generated:")) return "Generated";
    return "Node";
}

function nodeKindPriority(nodeId) {
    if (!nodeId) return 0;
    if (nodeId.startsWith("uisp:site:")) return 3;
    if (nodeId.startsWith("libreqos:generated:")) return 2;
    if (nodeId.startsWith("uisp:device:")) return 1;
    return 0;
}

function sortByName(left, right) {
    return (left?.node_name || "").localeCompare(right?.node_name || "");
}

function rememberNodeLabel(nodeId, nodeName) {
    const normalizedId = (nodeId || "").trim();
    const normalizedName = (nodeName || "").trim();
    if (!normalizedId || !normalizedName || topologyLabelById.has(normalizedId)) {
        return;
    }
    topologyLabelById.set(normalizedId, normalizedName);
}

function displayNameForNodeId(nodeId) {
    if (!nodeId) {
        return "";
    }
    if (nodeId === ROOT_NODE_ID) {
        return "Root";
    }
    return topologyLabelById.get(nodeId)
        || topologyNodeById.get(nodeId)?.node_name
        || treeNodeById.get(nodeId)?.name
        || nodeId;
}

function indexTopologyState(data) {
    topologyManagerState = data;
    topologyNodeById = new Map();
    topologyLabelById = new Map();
    topologyNodesSorted = Array.isArray(data?.nodes) ? [...data.nodes].sort(sortByName) : [];
    topologyNodesSorted.forEach((node) => {
        topologyNodeById.set(node.node_id, node);
        rememberNodeLabel(node.node_id, node.node_name);
        rememberNodeLabel(node.current_parent_node_id, node.current_parent_node_name);
        rememberNodeLabel(node.override_parent_node_id, node.override_parent_node_name);
        (node.allowed_parents || []).forEach((parent) => {
            rememberNodeLabel(parent.parent_node_id, parent.parent_node_name);
        });
    });
}

function indexNetworkTree(data) {
    networkTree = Array.isArray(data) ? data : [];
    treeNodeById = new Map();
    treeNodeByIndex = new Map();
    treeNodeIndexById = new Map();
    childrenByNodeId = new Map();
    treeNodeById.set(ROOT_NODE_ID, {
        id: ROOT_NODE_ID,
        name: "Root",
        immediate_parent: null,
    });

    networkTree.forEach(([index, node]) => {
        treeNodeByIndex.set(index, node);
        if (node?.id) {
            treeNodeById.set(node.id, node);
            treeNodeIndexById.set(node.id, index);
            rememberNodeLabel(node.id, node.name);
        }
    });

    networkTree.forEach(([, node]) => {
        const parentIndex = Number.isInteger(node.immediate_parent) ? node.immediate_parent : null;
        const parentNode = parentIndex !== null ? treeNodeByIndex.get(parentIndex) : null;
        const childId = node?.id || null;
        const parentId = parentNode?.id
            || (parentNode?.name === "Root" ? ROOT_NODE_ID : null);
        if (parentId && childId) {
            if (!childrenByNodeId.has(parentId)) {
                childrenByNodeId.set(parentId, []);
            }
            childrenByNodeId.get(parentId).push(childId);
        }
    });

    for (const children of childrenByNodeId.values()) {
        children.sort((leftId, rightId) => {
            const left = topologyNodeById.get(leftId) || treeNodeById.get(leftId);
            const right = topologyNodeById.get(rightId) || treeNodeById.get(rightId);
            return (left?.node_name || left?.name || "").localeCompare(right?.node_name || right?.name || "");
        });
    }
}

function treeParentNode(nodeId) {
    if (nodeId === ROOT_NODE_ID) {
        return null;
    }
    const nodeIndex = treeNodeIndexById.get(nodeId);
    const node = nodeIndex !== undefined ? treeNodeByIndex.get(nodeIndex) : null;
    const parentIndex = Number.isInteger(node?.immediate_parent) ? node.immediate_parent : null;
    return parentIndex !== null ? treeNodeByIndex.get(parentIndex) || null : null;
}

function treeParentNodeId(nodeId) {
    const parent = treeParentNode(nodeId);
    if (!parent) {
        return null;
    }
    return parent.id || (parent.name === "Root" ? ROOT_NODE_ID : null);
}

function canSelectNodeId(nodeId) {
    if (!nodeId) {
        return false;
    }
    return nodeId === ROOT_NODE_ID
        || topologyNodeById.has(nodeId)
        || treeNodeById.has(nodeId)
        || topologyLabelById.has(nodeId);
}

function synthesizeContextMeta(nodeId) {
    if (nodeId === ROOT_NODE_ID) {
        return {
            node_id: ROOT_NODE_ID,
            node_name: "Root",
            current_parent_node_id: null,
            current_parent_node_name: null,
            can_move: false,
            allowed_parents: [],
            has_override: false,
            override_parent_node_id: null,
            override_parent_node_name: null,
            override_mode: "auto",
            override_attachment_preference_ids: [],
            override_attachment_preference_names: [],
            warnings: [
                "Root is available for map navigation, but it is context only and cannot be rehomed.",
            ],
        };
    }
    const treeNode = treeNodeById.get(nodeId);
    if (!treeNode) {
        return null;
    }
    const parent = treeParentNode(nodeId);
    return {
        node_id: nodeId,
        node_name: displayNameForNodeId(nodeId),
        current_parent_node_id: parent?.id || null,
        current_parent_node_name: parent?.name || null,
        can_move: false,
        allowed_parents: [],
        has_override: false,
        override_parent_node_id: null,
        override_parent_node_name: null,
        override_mode: "auto",
        override_attachment_preference_ids: [],
        override_attachment_preference_names: [],
        warnings: [
            "This node is available for map navigation, but it is upstream context only and cannot be rehomed here.",
        ],
    };
}

function synthesizeStandaloneContextMeta(nodeId) {
    if (!topologyLabelById.has(nodeId)) {
        return null;
    }
    return {
        node_id: nodeId,
        node_name: displayNameForNodeId(nodeId),
        current_parent_node_id: ROOT_NODE_ID,
        current_parent_node_name: "Root",
        can_move: false,
        allowed_parents: [],
        has_override: false,
        override_parent_node_id: null,
        override_parent_node_name: null,
        override_mode: "auto",
        override_attachment_preference_ids: [],
        override_attachment_preference_names: [],
        warnings: [
            "This upstream context node is available for navigation only. Topology Manager cannot rehome it here.",
        ],
    };
}

function selectedMeta() {
    if (!selectedNodeId) {
        return null;
    }
    return topologyNodeById.get(selectedNodeId)
        || synthesizeContextMeta(selectedNodeId)
        || synthesizeStandaloneContextMeta(selectedNodeId)
        || null;
}

function selectedTreeNode() {
    return selectedNodeId ? treeNodeById.get(selectedNodeId) || null : null;
}

function selectNodeFromMap(nodeId) {
    if (!nodeId) {
        return;
    }
    setSelectedNode(nodeId);
}

function currentPathIds(nodeId) {
    const path = [];
    const seen = new Set();
    let cursor = nodeId;
    while (cursor && !seen.has(cursor)) {
        seen.add(cursor);
        const node = topologyNodeById.get(cursor);
        path.unshift(cursor);
        cursor = node?.current_parent_node_id || treeParentNodeId(cursor) || null;
    }
    if (path.length > 0 && path[0] !== ROOT_NODE_ID && !topologyNodeById.has(path[0]) && !treeNodeById.has(path[0])) {
        path.unshift(ROOT_NODE_ID);
    }
    return path;
}

function proposedPathIds(nodeId, parentId) {
    if (!nodeId || !parentId) {
        return currentPathIds(nodeId);
    }
    const parentPath = currentPathIds(parentId);
    if (parentPath[parentPath.length - 1] === nodeId) {
        return parentPath;
    }
    return [...parentPath, nodeId];
}

function descendantCount(nodeId) {
    const seen = new Set();
    const stack = [...(childrenByNodeId.get(nodeId) || [])];
    let count = 0;
    while (stack.length > 0) {
        const current = stack.pop();
        if (!current || seen.has(current)) {
            continue;
        }
        seen.add(current);
        count += 1;
        const children = childrenByNodeId.get(current) || [];
        children.forEach((childId) => stack.push(childId));
    }
    return count;
}

function childPreviewScore(nodeId) {
    const meta = topologyNodeById.get(nodeId) || null;
    const descendants = descendantCount(nodeId);
    const isMovable = meta?.can_move ? 1 : 0;
    const legalParentCount = meta?.can_move ? (meta.allowed_parents || []).length : 0;
    return {
        descendants,
        isMovable,
        legalParentCount,
        displayName: displayNameForNodeId(nodeId).toLowerCase(),
    };
}

function preferredChildPreviewNodes(nodeId) {
    const children = [...(childrenByNodeId.get(nodeId) || [])];
    const ranked = children
        .map((childId) => ({childId, ...childPreviewScore(childId)}))
        .sort((left, right) => {
            if ((left.descendants > 0 ? 1 : 0) !== (right.descendants > 0 ? 1 : 0)) {
                return (right.descendants > 0 ? 1 : 0) - (left.descendants > 0 ? 1 : 0);
            }
            if (left.isMovable !== right.isMovable) {
                return right.isMovable - left.isMovable;
            }
            if (left.descendants !== right.descendants) {
                return right.descendants - left.descendants;
            }
            if (left.legalParentCount !== right.legalParentCount) {
                return right.legalParentCount - left.legalParentCount;
            }
            return left.displayName.localeCompare(right.displayName);
        });

    const branchChildren = ranked.filter((child) => child.descendants > 0 || child.isMovable);
    const chosen = (branchChildren.length > 0 ? branchChildren : ranked)
        .slice(0, MAX_CHILD_PREVIEW_NODES)
        .map((child) => child.childId);

    chosen.sort((leftId, rightId) =>
        displayNameForNodeId(leftId).localeCompare(displayNameForNodeId(rightId))
    );
    return chosen;
}

function rankedSearchMatches(rawQuery) {
    const query = (rawQuery || "").trim().toLowerCase();
    if (!query) {
        return [];
    }
    const matches = topologyNodesSorted.filter((node) =>
        node.node_name.toLowerCase().includes(query)
    );
    matches.sort((left, right) => {
        const leftName = left.node_name.toLowerCase();
        const rightName = right.node_name.toLowerCase();
        const leftExact = leftName === query ? 1 : 0;
        const rightExact = rightName === query ? 1 : 0;
        if (leftExact !== rightExact) {
            return rightExact - leftExact;
        }
        const leftStarts = leftName.startsWith(query) ? 1 : 0;
        const rightStarts = rightName.startsWith(query) ? 1 : 0;
        if (leftStarts !== rightStarts) {
            return rightStarts - leftStarts;
        }
        const leftWordStart = leftName.split(/[^a-z0-9]+/).some((part) => part.startsWith(query)) ? 1 : 0;
        const rightWordStart = rightName.split(/[^a-z0-9]+/).some((part) => part.startsWith(query)) ? 1 : 0;
        if (leftWordStart !== rightWordStart) {
            return rightWordStart - leftWordStart;
        }
        return compareSelectionCandidates(left, right);
    });
    return matches;
}

function clearSearchSuggestions() {
    searchSuggestionNodeIds = [];
    searchSuggestionIndex = -1;
    const container = document.getElementById("topologyManagerSearchSuggestions");
    if (container) {
        container.innerHTML = "";
        container.classList.add("d-none");
    }
}

function renderSearchSuggestions(rawQuery) {
    const container = document.getElementById("topologyManagerSearchSuggestions");
    if (!container) {
        return;
    }
    const matches = rankedSearchMatches(rawQuery).slice(0, MAX_SEARCH_SUGGESTIONS);
    searchSuggestionNodeIds = matches.map((node) => node.node_id);
    searchSuggestionIndex = matches.length > 0 ? 0 : -1;

    if (matches.length === 0) {
        container.innerHTML = "";
        container.classList.add("d-none");
        return;
    }

    container.innerHTML = matches.map((node, index) => `
        <button class="dropdown-item topology-manager-search-suggestion ${index === searchSuggestionIndex ? "active" : ""}" type="button" data-search-node-id="${escapeHtml(node.node_id)}">
            <span class="fw-semibold">${escapeHtml(node.node_name)}</span>
            <span class="small text-body-secondary">${escapeHtml(nodeKindLabel(node.node_id))}</span>
        </button>
    `).join("");
    container.classList.remove("d-none");
}

function updateSearchSuggestionHighlight() {
    const container = document.getElementById("topologyManagerSearchSuggestions");
    if (!container) {
        return;
    }
    const items = [...container.querySelectorAll("[data-search-node-id]")];
    items.forEach((item, index) => {
        item.classList.toggle("active", index === searchSuggestionIndex);
    });
}

function selectSearchSuggestion(nodeId) {
    if (!nodeId) {
        return;
    }
    const node = topologyNodeById.get(nodeId);
    if (!node) {
        return;
    }
    const searchInput = document.getElementById("topologyManagerNodeSearch");
    if (searchInput) {
        searchInput.value = node.node_name;
    }
    clearSearchSuggestions();
    setSelectedNode(nodeId);
}

function compareSelectionCandidates(left, right) {
    const leftAllowedCount = left.can_move ? (left.allowed_parents || []).length : -1;
    const rightAllowedCount = right.can_move ? (right.allowed_parents || []).length : -1;
    const leftDescendants = descendantCount(left.node_id);
    const rightDescendants = descendantCount(right.node_id);
    const leftBranchContext = leftDescendants > 0 ? 1 : 0;
    const rightBranchContext = rightDescendants > 0 ? 1 : 0;
    const leftMultiTarget = leftAllowedCount > 1 ? 1 : 0;
    const rightMultiTarget = rightAllowedCount > 1 ? 1 : 0;
    const leftKind = nodeKindPriority(left.node_id);
    const rightKind = nodeKindPriority(right.node_id);

    if ((left.has_override ? 1 : 0) !== (right.has_override ? 1 : 0)) {
        return (right.has_override ? 1 : 0) - (left.has_override ? 1 : 0);
    }
    if ((left.can_move ? 1 : 0) !== (right.can_move ? 1 : 0)) {
        return (right.can_move ? 1 : 0) - (left.can_move ? 1 : 0);
    }
    if (leftBranchContext !== rightBranchContext) {
        return rightBranchContext - leftBranchContext;
    }
    if (leftMultiTarget !== rightMultiTarget) {
        return rightMultiTarget - leftMultiTarget;
    }
    if (leftKind !== rightKind) {
        return rightKind - leftKind;
    }
    if (leftDescendants !== rightDescendants) {
        return rightDescendants - leftDescendants;
    }
    if (leftAllowedCount !== rightAllowedCount) {
        return rightAllowedCount - leftAllowedCount;
    }
    return sortByName(left, right);
}

function pickDefaultNodeId() {
    if (topologyNodesSorted.length === 0) {
        return null;
    }
    const ranked = [...topologyNodesSorted].sort(compareSelectionCandidates);
    return ranked[0]?.node_id || null;
}

function allowedParentsForSelected() {
    return [...(selectedMeta()?.allowed_parents || [])].sort((left, right) =>
        left.parent_node_name.localeCompare(right.parent_node_name)
    );
}

function proposedParentMeta() {
    const parentId = proposedParentId;
    if (!parentId) {
        return null;
    }
    return allowedParentsForSelected().find((entry) => entry.parent_node_id === parentId) || null;
}

function attachmentEditParentId(meta = selectedMeta()) {
    if (!meta) {
        return null;
    }
    const preferredId = meta.has_override
        ? (meta.override_parent_node_id || meta.current_parent_node_id || null)
        : (meta.current_parent_node_id || null);
    if (!preferredId) {
        return null;
    }
    return allowedParentsForSelected().some((entry) => entry.parent_node_id === preferredId)
        ? preferredId
        : null;
}

function attachmentEditParentMeta(meta = selectedMeta()) {
    const parentId = attachmentEditParentId(meta);
    if (!parentId) {
        return null;
    }
    return allowedParentsForSelected().find((entry) => entry.parent_node_id === parentId) || null;
}

function hasInspectableAttachmentChoices(meta = selectedMeta()) {
    return explicitAttachmentOptions(attachmentEditParentMeta(meta)).length > 1;
}

function attachmentOptionsForProposedParent() {
    return proposedParentMeta()?.attachment_options || [];
}

function explicitAttachmentOptions(parentMeta) {
    return (parentMeta?.attachment_options || []).filter((option) => option.attachment_id !== "auto");
}

function manualAttachmentParentId(meta = selectedMeta()) {
    if (!meta) {
        return null;
    }
    if (moveMode && proposedParentId) {
        return proposedParentId;
    }
    return attachmentEditParentId(meta);
}

function manualAttachmentParentMeta(meta = selectedMeta()) {
    const parentId = manualAttachmentParentId(meta);
    if (!parentId) {
        return null;
    }
    return allowedParentsForSelected().find((entry) => entry.parent_node_id === parentId) || null;
}

function hasManualAttachmentGroup(parentMeta) {
    const explicit = explicitAttachmentOptions(parentMeta);
    return explicit.length > 0 && explicit.every((option) => option.attachment_kind === "manual");
}

function cloneManualAttachmentRows(parentMeta) {
    return explicitAttachmentOptions(parentMeta).map((option) => ({
        attachment_id: option.attachment_id || "",
        attachment_name: option.attachment_name || "",
        capacity_mbps: String(option.capacity_mbps ?? ""),
        local_probe_ip: option.local_probe_ip || "",
        remote_probe_ip: option.remote_probe_ip || "",
        probe_enabled: !!option.probe_enabled,
    }));
}

function blankManualAttachmentRow() {
    return {
        attachment_id: "",
        attachment_name: "",
        capacity_mbps: "",
        local_probe_ip: "",
        remote_probe_ip: "",
        probe_enabled: false,
    };
}

function formatUnixTimestamp(value) {
    if (!value) {
        return "Not scheduled";
    }
    try {
        return new Date(value * 1000).toLocaleString();
    } catch (_) {
        return String(value);
    }
}

function attachmentHealthBadge(option) {
    const status = option?.health_status || "disabled";
    if (status === "suppressed") {
        return `<span class="badge text-bg-danger">Suppressed</span>`;
    }
    if (status === "probe_unavailable") {
        return `<span class="badge text-bg-secondary">Probe Unavailable</span>`;
    }
    if (status === "healthy") {
        return `<span class="badge text-bg-success">Healthy</span>`;
    }
    return `<span class="badge text-bg-secondary">Probe Disabled</span>`;
}

function attachmentHealthDetails(option) {
    const rows = [];
    if (option?.health_reason) {
        rows.push(escapeHtml(option.health_reason));
    }
    if (option?.suppressed_until_unix) {
        rows.push(`Suppressed until ${escapeHtml(formatUnixTimestamp(option.suppressed_until_unix))}`);
    }
    if (option?.capacity_mbps) {
        rows.push(`${escapeHtml(option.capacity_mbps)} Mbps`);
    }
    if (option?.local_probe_ip || option?.remote_probe_ip) {
        rows.push(`${escapeHtml(option.local_probe_ip || "?")} ↔ ${escapeHtml(option.remote_probe_ip || "?")}`);
    }
    return rows.join(" · ");
}

function proposalMatchesSavedOverride(meta = selectedMeta()) {
    if (!meta) {
        return !proposedParentId;
    }
    if (!meta.has_override) {
        return proposedParentId === (meta.current_parent_node_id || null)
            && proposedMode === "auto"
            && (proposedAttachmentIds || []).length === 0;
    }
    const idsMatch = JSON.stringify(meta.override_attachment_preference_ids || [])
        === JSON.stringify(proposedAttachmentIds || []);
    return meta.override_parent_node_id === proposedParentId
        && (meta.override_mode || "auto") === proposedMode
        && idsMatch;
}

function proposalIsValid() {
    if (!proposedParentId) {
        return false;
    }
    if (proposedMode === "preferred_order") {
        return proposedAttachmentIds.length > 0;
    }
    return true;
}

function initializeProposalFromSaved(meta = selectedMeta()) {
    if (meta?.has_override) {
        proposedParentId = meta.override_parent_node_id || null;
        proposedMode = meta.override_mode || "auto";
        proposedAttachmentIds = [...(meta.override_attachment_preference_ids || [])];
    } else {
        proposedParentId = null;
        proposedMode = "auto";
        proposedAttachmentIds = [];
    }
}

function clampMapScale(scale) {
    return Math.min(MAP_ZOOM_MAX, Math.max(MAP_ZOOM_MIN, scale));
}

function resetMapView() {
    mapView = {scale: 1, x: 0, y: 0};
}

function mapPointFromClient(svg, clientX, clientY) {
    if (!svg) {
        return {x: MAP_VIEWBOX.width / 2, y: MAP_VIEWBOX.height / 2};
    }
    const rect = svg.getBoundingClientRect();
    if (!rect.width || !rect.height) {
        return {x: MAP_VIEWBOX.width / 2, y: MAP_VIEWBOX.height / 2};
    }
    return {
        x: ((clientX - rect.left) / rect.width) * MAP_VIEWBOX.width,
        y: ((clientY - rect.top) / rect.height) * MAP_VIEWBOX.height,
    };
}

function setMapZoomAroundPoint(nextScale, anchor) {
    const clampedScale = clampMapScale(nextScale);
    if (Math.abs(clampedScale - mapView.scale) < 0.001) {
        return;
    }
    const factor = clampedScale / mapView.scale;
    mapView = {
        scale: clampedScale,
        x: anchor.x - (factor * (anchor.x - mapView.x)),
        y: anchor.y - (factor * (anchor.y - mapView.y)),
    };
}

function mapViewportTransform() {
    return `translate(${mapView.x} ${mapView.y}) scale(${mapView.scale})`;
}

function showWarnings(messages, flash = null) {
    const container = document.getElementById("topologyManagerWarnings");
    if (!container) return;

    const rows = [];
    if (flash) {
        rows.push(`<div class="alert alert-info py-2 mb-2">${escapeHtml(flash)}</div>`);
    }
    (messages || []).forEach((message) => {
        rows.push(`<div class="alert alert-warning py-2 mb-2">${escapeHtml(message)}</div>`);
    });
    container.innerHTML = rows.join("");
}

function renderModeBanner() {
    const container = document.getElementById("topologyManagerModeBanner");
    if (!container) return;

    const meta = selectedMeta();
    if (!meta) {
        container.innerHTML = "<div class='alert alert-secondary py-2 mb-0'>Select a branch to inspect it. Start a move only when you want to change its parentage.</div>";
        return;
    }

    if (!meta.can_move) {
        container.innerHTML = `
            <div class="alert alert-secondary py-2 mb-0">
                <strong>Inspect mode.</strong> ${escapeHtml(meta.node_name)} is read-only. You can still click through upstream and downstream context in the preview, but this node cannot be rehomed here.
            </div>
        `;
        return;
    }

    if (!moveMode && !attachmentEditMode) {
        const attachmentAction = hasInspectableAttachmentChoices(meta)
            ? " To tune radio preference without changing parentage, use <strong>Edit Attachment Preference</strong> in the Details panel."
            : "";
        container.innerHTML = `
            <div class="alert alert-secondary py-2 mb-0">
                <strong>Inspect mode.</strong> Click any node in the preview to navigate context. To change parentage for ${escapeHtml(meta.node_name)}, use <strong>Start Move</strong> in the Details panel.${attachmentAction} Until then, the preview is context only.
            </div>
        `;
        return;
    }

    if (attachmentEditMode) {
        const attachmentParentName = attachmentEditParentMeta(meta)?.parent_node_name || "current parent";
        container.innerHTML = `
            <div class="alert alert-primary py-2 mb-0">
                <strong>Attachment edit mode.</strong> Adjust attachment/radio preference for <strong>${escapeHtml(attachmentParentName)}</strong> without changing parentage.
            </div>
        `;
        return;
    }

    if (manualAttachmentEditMode) {
        const parentName = manualAttachmentParentMeta(meta)?.parent_node_name || "selected parent";
        container.innerHTML = `
            <div class="alert alert-primary py-2 mb-0">
                <strong>Manual attachment group mode.</strong> Define explicit parallel attachments for <strong>${escapeHtml(parentName)}</strong>, including capacity, probe IPs, and probe opt-in.
            </div>
        `;
        return;
    }

    const proposedParentName = proposedParentMeta()?.parent_node_name || "none selected yet";
    container.innerHTML = `
        <div class="alert alert-primary py-2 mb-0">
            <strong>Move mode.</strong> Green nodes are allowed parents for ${escapeHtml(meta.node_name)}. Choose one from the target cards or drag the selected branch onto a green target. Current choice: <strong>${escapeHtml(proposedParentName)}</strong>.
        </div>
    `;
}

function updateHeader() {
    const summary = document.getElementById("topologyManagerSummary");
    const source = document.getElementById("topologyManagerSource");
    if (!summary || !source) return;

    const movableCount = topologyNodesSorted.filter((node) => node.can_move).length;
    const overrideCount = topologyNodesSorted.filter((node) => node.has_override).length;
    const selected = selectedMeta();
    summary.textContent = selected
        ? `${selected.node_name} selected, ${movableCount} movable branches, ${overrideCount} saved moves`
        : `${movableCount} movable branches, ${overrideCount} saved moves`;
    source.textContent = topologyManagerState?.source
        ? `Source: ${topologyManagerState.source} (schema ${topologyManagerState.schema_version})`
        : "No topology editor source is currently available";
}

function renderHierarchyPath(containerId, pathIds, options = {}) {
    const container = document.getElementById(containerId);
    if (!container) return;
    if (!pathIds || pathIds.length === 0) {
        container.innerHTML = "<span class='text-body-secondary'>No hierarchy path available.</span>";
        return;
    }

    const html = [];
    pathIds.forEach((nodeId, index) => {
        const label = displayNameForNodeId(nodeId);
        const chipClass = options.highlightNodeId === nodeId
            ? "topology-manager-chip border-warning-subtle bg-warning-subtle text-warning-emphasis"
            : "topology-manager-chip";
        html.push(`<span class="${chipClass}">${escapeHtml(label)}</span>`);
        if (index < pathIds.length - 1) {
            html.push("<span class='topology-manager-path-sep'><i class='fa fa-chevron-right'></i></span>");
        }
    });
    container.innerHTML = html.join("");
}

function renderHierarchyPanel() {
    const currentIds = currentPathIds(selectedNodeId);
    const meta = selectedMeta();
    const savedOverrideIds = meta?.has_override
        ? proposedPathIds(selectedNodeId, meta.override_parent_node_id)
        : currentIds;
    const proposedIds = moveMode && proposedParentId
        ? proposedPathIds(selectedNodeId, proposedParentId)
        : savedOverrideIds;
    renderHierarchyPath("topologyManagerCurrentHierarchy", currentIds, {
        highlightNodeId: selectedNodeId,
    });
    renderHierarchyPath("topologyManagerProposedHierarchy", proposedIds, {
        highlightNodeId: selectedNodeId,
    });

    const badge = document.getElementById("topologyManagerPendingBadge");
    if (badge) {
        if (!selectedNodeId) {
            badge.className = "badge rounded-pill text-bg-warning d-none";
        } else if ((moveMode || attachmentEditMode) && proposedParentId && !proposalMatchesSavedOverride(meta)) {
            badge.className = "badge rounded-pill text-bg-warning";
            badge.textContent = "Unsaved";
        } else if (moveMode) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Move Mode";
        } else if (attachmentEditMode) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Attachment Mode";
        } else if (manualAttachmentEditMode) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Manual Group";
        } else if (meta?.has_override) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Saved Override";
        } else {
            badge.className = "badge rounded-pill text-bg-warning d-none";
        }
    }

    const details = document.getElementById("topologyManagerHierarchyMeta");
    if (!details) return;
    details.innerHTML = "";
}

function renderTargetCards(meta) {
    const allowedParents = allowedParentsForSelected();
    if (allowedParents.length === 0) {
        return "<div class='topology-manager-empty'>No valid rehome targets are currently available for this branch.</div>";
    }

    const currentParentId = meta.current_parent_node_id;
    return `
        <div class="topology-manager-target-grid">
            ${allowedParents.map((parent) => {
                const isCurrent = parent.parent_node_id === currentParentId;
                const isActive = parent.parent_node_id === proposedParentId;
                const attachmentCount = parent.attachment_options.filter((option) => option.attachment_id !== "auto").length;
                const cardClasses = [
                    "topology-manager-target-card",
                    isCurrent ? "current" : "",
                    isActive ? "active" : "",
                ].filter(Boolean).join(" ");
                return `
                    <button class="${cardClasses} text-start w-100" type="button" data-parent-target-id="${escapeHtml(parent.parent_node_id)}">
                        <div class="d-flex align-items-center justify-content-between gap-2 mb-1">
                            <strong>${escapeHtml(parent.parent_node_name)}</strong>
                            ${isCurrent ? "<span class='badge text-bg-primary'>Current</span>" : ""}
                        </div>
                        <div class="small text-body-secondary">
                            ${attachmentCount > 0
                                ? `${attachmentCount} explicit attachment option${attachmentCount === 1 ? "" : "s"}`
                                : "Auto attachment only"}
                        </div>
                        <div class="d-flex flex-wrap gap-1 mt-2">
                            ${parent.all_attachments_suppressed ? "<span class='badge text-bg-danger'>All Attachments Suppressed</span>" : ""}
                            ${parent.has_probe_unavailable_attachments ? "<span class='badge text-bg-secondary'>Probe Unavailable</span>" : ""}
                            ${hasManualAttachmentGroup(parent) ? "<span class='badge text-bg-info'>Manual Group</span>" : ""}
                        </div>
                    </button>
                `;
            }).join("")}
        </div>
    `;
}

function renderAttachmentEditor(meta) {
    if (!proposedParentId) {
        return "<div class='text-body-secondary'>Choose a new parent first. Attachment ranking only appears after a target is selected.</div>";
    }

    const options = attachmentOptionsForProposedParent();
    const explicitOptions = options.filter((option) => option.attachment_id !== "auto");
    const rankedIds = proposedAttachmentIds.filter((attachmentId) =>
        explicitOptions.some((option) => option.attachment_id === attachmentId)
    );

    return `
        <div class="d-flex flex-column gap-3">
            <div>
                <div class="small text-uppercase text-body-secondary mb-2">Attachment Mode</div>
                <div class="form-check">
                    <input class="form-check-input" type="radio" name="topologyAttachmentMode" id="topologyAttachmentModeAuto" value="auto" ${proposedMode === "auto" ? "checked" : ""}>
                    <label class="form-check-label" for="topologyAttachmentModeAuto">Auto / Default</label>
                </div>
                <div class="form-check">
                    <input class="form-check-input" type="radio" name="topologyAttachmentMode" id="topologyAttachmentModeManual" value="preferred_order" ${proposedMode === "preferred_order" ? "checked" : ""} ${explicitOptions.length === 0 ? "disabled" : ""}>
                    <label class="form-check-label" for="topologyAttachmentModeManual">Rank explicit attachment preferences</label>
                </div>
            </div>

            <div>
                <div class="small text-uppercase text-body-secondary mb-2">Available Attachments</div>
                ${explicitOptions.length === 0
                    ? "<div class='text-body-secondary'>This target currently exposes only Auto / Default attachment behavior.</div>"
                    : `
                        <div class="list-group topology-manager-rank-list">
                            ${explicitOptions.map((option) => {
                                const isRanked = rankedIds.includes(option.attachment_id);
                                const isEffective = !!option.effective_selected;
                                return `
                                    <div class="list-group-item topology-manager-attachment-row">
                                        <div class="d-flex align-items-center justify-content-between gap-2">
                                            <div>
                                                <div class="d-flex flex-wrap align-items-center gap-2 mb-1">
                                                    <div class="fw-semibold">${escapeHtml(option.attachment_name)}</div>
                                                    ${attachmentHealthBadge(option)}
                                                    ${isEffective ? "<span class='badge text-bg-primary'>Effective</span>" : ""}
                                                    ${option.probe_enabled ? "<span class='badge text-bg-info'>Probe On</span>" : "<span class='badge text-bg-secondary'>Probe Off</span>"}
                                                </div>
                                                <div class="small text-body-secondary">${escapeHtml(option.attachment_kind || "attachment")}</div>
                                                <div class="small text-body-secondary">${attachmentHealthDetails(option)}</div>
                                            </div>
                                            <div class="d-flex flex-wrap gap-2 justify-content-end">
                                                <button class="btn btn-sm ${isRanked ? "btn-outline-secondary" : "btn-outline-primary"}" type="button" data-add-attachment-id="${escapeHtml(option.attachment_id)}">
                                                    ${isRanked ? "Added" : "Add"}
                                                </button>
                                                ${option.pair_id
                                                    ? `<button class="btn btn-sm ${option.probe_enabled ? "btn-outline-secondary" : "btn-outline-info"}" type="button" data-probe-policy-pair-id="${escapeHtml(option.pair_id)}" data-probe-policy-enabled="${option.probe_enabled ? "false" : "true"}">
                                                        ${option.probe_enabled ? "Disable Probe" : "Enable Probe"}
                                                    </button>`
                                                    : ""}
                                            </div>
                                        </div>
                                    </div>
                                `;
                            }).join("")}
                        </div>
                    `}
            </div>

            <div>
                <div class="small text-uppercase text-body-secondary mb-2">Preference Order</div>
                ${rankedIds.length === 0
                    ? "<div class='text-body-secondary'>No explicit attachment preferences selected.</div>"
                    : `
                        <div class="list-group topology-manager-rank-list">
                            ${rankedIds.map((attachmentId, index) => {
                                const option = explicitOptions.find((entry) => entry.attachment_id === attachmentId);
                                if (!option) {
                                    return "";
                                }
                                return `
                                    <div class="list-group-item">
                                        <div class="d-flex align-items-center justify-content-between gap-2">
                                            <div>
                                                <div class="fw-semibold">Preference ${index + 1}: ${escapeHtml(option.attachment_name)}</div>
                                                <div class="small text-body-secondary">${escapeHtml(option.attachment_kind || "attachment")}</div>
                                            </div>
                                            <div class="btn-group btn-group-sm">
                                                <button class="btn btn-outline-secondary" type="button" data-rank-up-id="${escapeHtml(option.attachment_id)}"><i class="fa fa-arrow-up"></i></button>
                                                <button class="btn btn-outline-secondary" type="button" data-rank-down-id="${escapeHtml(option.attachment_id)}"><i class="fa fa-arrow-down"></i></button>
                                                <button class="btn btn-outline-danger" type="button" data-remove-attachment-id="${escapeHtml(option.attachment_id)}"><i class="fa fa-xmark"></i></button>
                                            </div>
                                        </div>
                                    </div>
                                `;
                            }).join("")}
                        </div>
                    `}
            </div>
        </div>
    `;
}

function renderAttachmentHealthList(parentMeta, options = {}) {
    const explicitOptions = explicitAttachmentOptions(parentMeta);
    if (explicitOptions.length === 0) {
        return "<div class='text-body-secondary'>This logical parent currently exposes only Auto / Default attachment behavior.</div>";
    }

    const showProbeButtons = options.showProbeButtons !== false;
    return `
        <div class="list-group topology-manager-rank-list">
            ${explicitOptions.map((option) => `
                <div class="list-group-item topology-manager-attachment-row">
                    <div class="d-flex align-items-center justify-content-between gap-2">
                        <div>
                            <div class="d-flex flex-wrap align-items-center gap-2 mb-1">
                                <div class="fw-semibold">${escapeHtml(option.attachment_name)}</div>
                                ${attachmentHealthBadge(option)}
                                ${option.effective_selected ? "<span class='badge text-bg-primary'>Effective</span>" : ""}
                                ${option.probe_enabled ? "<span class='badge text-bg-info'>Probe On</span>" : "<span class='badge text-bg-secondary'>Probe Off</span>"}
                            </div>
                            <div class="small text-body-secondary">${escapeHtml(option.attachment_kind || "attachment")}</div>
                            <div class="small text-body-secondary">${attachmentHealthDetails(option)}</div>
                        </div>
                        ${showProbeButtons && option.pair_id
                            ? `<button class="btn btn-sm ${option.probe_enabled ? "btn-outline-secondary" : "btn-outline-info"}" type="button" data-probe-policy-pair-id="${escapeHtml(option.pair_id)}" data-probe-policy-enabled="${option.probe_enabled ? "false" : "true"}">
                                ${option.probe_enabled ? "Disable Probe" : "Enable Probe"}
                            </button>`
                            : ""}
                    </div>
                </div>
            `).join("")}
        </div>
    `;
}

function renderManualAttachmentGroupSection(meta) {
    const parent = manualAttachmentParentMeta(meta);
    if (!parent || !meta?.can_move) {
        return "";
    }

    const explicit = explicitAttachmentOptions(parent);
    const hasManualGroup = hasManualAttachmentGroup(parent);
    const rowDraft = manualAttachmentEditMode
        ? manualAttachmentDraftRows
        : cloneManualAttachmentRows(parent);
    const summary = hasManualGroup
        ? "Using an operator-defined manual attachment group for this logical parent."
        : "Using integration-emitted attachment options. Create a manual group here only when you need to model explicit parallel paths yourself.";

    if (!manualAttachmentEditMode) {
        return `
            <div class="d-flex flex-column gap-2">
                <div class="small text-uppercase text-body-secondary">Manual Attachment Group</div>
                <div class="small text-body-secondary">
                    Parent: <strong>${escapeHtml(parent.parent_node_name)}</strong><br>
                    ${escapeHtml(summary)}<br>
                    Explicit attachments currently visible: <strong>${explicit.length}</strong>
                </div>
                <div class="d-flex flex-wrap gap-2">
                    <button class="btn btn-outline-primary" id="topologyManagerEditManualAttachments" type="button">
                        <i class="fa fa-pen-to-square"></i> ${hasManualGroup ? "Edit Manual Attachment Group" : "Create Manual Attachment Group"}
                    </button>
                    ${hasManualGroup
                        ? `<button class="btn btn-outline-danger" id="topologyManagerClearManualAttachments" type="button">
                            <i class="fa fa-trash"></i> Clear Manual Attachment Group
                        </button>`
                        : ""}
                </div>
            </div>
        `;
    }

    return `
        <div class="d-flex flex-column gap-3">
            <div>
                <div class="small text-uppercase text-body-secondary mb-2">Manual Attachment Group</div>
                <div class="small text-body-secondary">
                    Define between 2 and 8 explicit paths for <strong>${escapeHtml(parent.parent_node_name)}</strong>. Array order becomes preference order when you rank attachments manually.
                </div>
            </div>

            <div class="d-flex flex-column gap-3">
                ${rowDraft.map((row, index) => `
                    <div class="topology-manager-manual-row">
                        <div class="d-flex align-items-center justify-content-between gap-2 mb-2">
                            <strong>Attachment ${index + 1}</strong>
                            <button class="btn btn-sm btn-outline-danger" type="button" data-remove-manual-row="${index}" ${rowDraft.length <= 2 ? "disabled" : ""}>
                                <i class="fa fa-trash"></i>
                            </button>
                        </div>
                        <div class="row g-2">
                            <div class="col-12">
                                <label class="form-label small mb-1">Attachment Name</label>
                                <input class="form-control form-control-sm" type="text" data-manual-row-index="${index}" data-manual-field="attachment_name" value="${escapeHtml(row.attachment_name)}" placeholder="WavePro-MREToRochester">
                            </div>
                            <div class="col-12">
                                <label class="form-label small mb-1">Attachment ID</label>
                                <input class="form-control form-control-sm" type="text" data-manual-row-index="${index}" data-manual-field="attachment_id" value="${escapeHtml(row.attachment_id)}" placeholder="manual:site-a-site-b:wavepro">
                            </div>
                            <div class="col-md-4">
                                <label class="form-label small mb-1">Capacity (Mbps)</label>
                                <input class="form-control form-control-sm" type="number" min="1" data-manual-row-index="${index}" data-manual-field="capacity_mbps" value="${escapeHtml(row.capacity_mbps)}" placeholder="500">
                            </div>
                            <div class="col-md-4">
                                <label class="form-label small mb-1">Local Probe IP</label>
                                <input class="form-control form-control-sm" type="text" data-manual-row-index="${index}" data-manual-field="local_probe_ip" value="${escapeHtml(row.local_probe_ip)}" placeholder="10.0.0.10">
                            </div>
                            <div class="col-md-4">
                                <label class="form-label small mb-1">Remote Probe IP</label>
                                <input class="form-control form-control-sm" type="text" data-manual-row-index="${index}" data-manual-field="remote_probe_ip" value="${escapeHtml(row.remote_probe_ip)}" placeholder="10.0.0.11">
                            </div>
                            <div class="col-12">
                                <div class="form-check mt-1">
                                    <input class="form-check-input" type="checkbox" id="topologyManualProbeEnabled${index}" data-manual-row-index="${index}" data-manual-field="probe_enabled" ${row.probe_enabled ? "checked" : ""}>
                                    <label class="form-check-label" for="topologyManualProbeEnabled${index}">Enable health probing for this attachment pair</label>
                                </div>
                            </div>
                        </div>
                    </div>
                `).join("")}
            </div>

            <div class="d-flex flex-wrap gap-2">
                <button class="btn btn-outline-secondary" id="topologyManagerAddManualAttachment" type="button" ${rowDraft.length >= 8 ? "disabled" : ""}>
                    <i class="fa fa-plus"></i> Add Attachment
                </button>
                <button class="btn btn-primary" id="topologyManagerSaveManualAttachments" type="button">
                    <i class="fa fa-save"></i> Save Manual Attachment Group
                </button>
                ${hasManualGroup
                    ? `<button class="btn btn-outline-danger" id="topologyManagerClearManualAttachments" type="button">
                        <i class="fa fa-trash"></i> Clear Manual Attachment Group
                    </button>`
                    : ""}
                <button class="btn btn-outline-secondary" id="topologyManagerCancelManualAttachments" type="button">
                    <i class="fa fa-ban"></i> Cancel
                </button>
            </div>
        </div>
    `;
}

function renderDetailsPanel() {
    const container = document.getElementById("topologyManagerDetails");
    const clearSavedButton = document.getElementById("topologyManagerClearSaved");
    if (!container || !clearSavedButton) {
        return;
    }

    const meta = selectedMeta();
    const treeNode = selectedTreeNode();
    if (!meta) {
        clearSavedButton.disabled = true;
        container.innerHTML = "<div class='topology-manager-empty'>Select a branch from the map or search box to begin.</div>";
        return;
    }

    clearSavedButton.disabled = !meta.has_override;
    const currentParent = meta.current_parent_node_name || "Root / none";
    const legalParentCount = allowedParentsForSelected().length;
    const savedOverrideText = meta.has_override
        ? `${escapeHtml(meta.override_parent_node_name || "")}${meta.override_mode === "preferred_order" && meta.override_attachment_preference_names.length > 0 ? ` via ${escapeHtml(meta.override_attachment_preference_names.join(" → "))}` : meta.override_mode === "auto" ? " (Auto)" : ""}`
        : "None";
    const attachmentParent = attachmentEditParentMeta(meta);
    const attachmentOptionCount = explicitAttachmentOptions(attachmentParent).length;
    const showAttachmentInspectEditor = !moveMode && attachmentEditMode && !!attachmentParent;
    const saveDisabled = !proposalIsValid() || proposalMatchesSavedOverride(meta);
    const editingDisabled = !meta.can_move;
    const currentAttachment = meta.current_attachment_name || "Auto / integration default";
    const preferredAttachment = meta.preferred_attachment_name || "Auto / integration default";
    const effectiveAttachment = meta.effective_attachment_name || "Auto / integration default";

    container.innerHTML = `
        ${(meta.warnings || []).map((warning) => `<div class="alert alert-warning py-2 mb-0">${escapeHtml(warning)}</div>`).join("")}

        <div class="d-flex flex-column gap-2">
            <div class="small text-uppercase text-body-secondary">Selected Branch</div>
            <div class="d-flex align-items-center justify-content-between gap-2">
                <div>
                    <div class="fw-semibold fs-5">${escapeHtml(meta.node_name)}</div>
                    <div class="small text-body-secondary">${escapeHtml(nodeKindLabel(meta.node_id))}</div>
                </div>
                <span class="badge ${meta.can_move ? "text-bg-info" : "text-bg-secondary"}">${meta.can_move ? "Movable" : "Read Only"}</span>
            </div>
            <div class="small text-body-secondary">
                Current parent: <strong>${escapeHtml(currentParent)}</strong><br>
                Saved override: <strong>${savedOverrideText}</strong><br>
                Current attachment: <strong>${escapeHtml(currentAttachment)}</strong><br>
                Preferred attachment: <strong>${escapeHtml(preferredAttachment)}</strong><br>
                Effective attachment: <strong>${escapeHtml(effectiveAttachment)}</strong><br>
                Descendants in branch: <strong>${descendantCount(meta.node_id)}</strong><br>
                Legal parent targets: <strong>${legalParentCount}</strong>
            </div>
            ${treeNode?.runtime_virtualized ? "<div class='small text-warning-emphasis'>Runtime virtualized in Bakery right now.</div>" : ""}
        </div>

        ${editingDisabled
            ? "<div class='alert alert-secondary py-2 mb-0'>This node is currently read-only in Topology Manager. Select a movable branch to rehome it.</div>"
            : `
                <div class="alert ${moveMode ? "alert-primary" : "alert-secondary"} py-2 mb-0">
                    ${moveMode
                        ? "Step 1: choose a legal parent below or drag the selected branch onto a green target. Step 2: adjust attachment preference if needed. Step 3: save the move."
                        : attachmentEditMode
                            ? "Edit attachment preference for the current logical parent, then save. This does not change parentage."
                            : manualAttachmentEditMode
                                ? "Define explicit parallel attachments for this logical parent, then save the manual group. This changes attachment modeling, not logical parentage."
                            : "Inspect mode only. The preview on the left shows context, not an always-active editor. Click Start Move when you want to change parentage."}
                </div>

                ${moveMode
                    ? `
                        <div>
                            <div class="small text-uppercase text-body-secondary mb-2">Choose New Parent</div>
                            ${renderTargetCards(meta)}
                        </div>

                        <div>
                            <div class="small text-uppercase text-body-secondary mb-2">Attachment / Radio Preferences</div>
                            ${renderAttachmentEditor(meta)}
                        </div>

                        <div class="d-flex flex-wrap gap-2">
                            <button class="btn btn-primary" id="topologyManagerSave" type="button" ${saveDisabled ? "disabled" : ""}>
                                <i class="fa fa-save"></i> Save Move
                            </button>
                            <button class="btn btn-outline-secondary" id="topologyManagerResetInline" type="button">
                                <i class="fa fa-rotate-left"></i> Reset to Saved
                            </button>
                            <button class="btn btn-outline-secondary" id="topologyManagerCancelMove" type="button">
                                <i class="fa fa-ban"></i> Cancel Move
                            </button>
                        </div>
                    `
                    : `
                        ${attachmentParent
                            ? `
                                <div class="d-flex flex-column gap-2">
                                    <div class="small text-uppercase text-body-secondary">Current Attachment Preference</div>
                                    <div class="small text-body-secondary">
                                        Parent: <strong>${escapeHtml(attachmentParent.parent_node_name)}</strong><br>
                                        ${attachmentOptionCount > 0
                                            ? `Explicit attachment options: <strong>${attachmentOptionCount}</strong>`
                                            : "This parent currently exposes only Auto / Default attachment behavior."}
                                    </div>
                                </div>
                            `
                            : ""}
                        ${attachmentParent
                            ? `
                                <div>
                                    <div class="small text-uppercase text-body-secondary mb-2">Attachment Health</div>
                                    ${renderAttachmentHealthList(attachmentParent)}
                                </div>
                            `
                            : ""}
                        ${showAttachmentInspectEditor
                            ? `
                                <div>
                                    <div class="small text-uppercase text-body-secondary mb-2">Attachment / Radio Preferences</div>
                                    ${renderAttachmentEditor(meta)}
                                </div>

                                <div class="d-flex flex-wrap gap-2">
                                    <button class="btn btn-primary" id="topologyManagerSave" type="button" ${saveDisabled ? "disabled" : ""}>
                                        <i class="fa fa-save"></i> Save Attachment Preference
                                    </button>
                                    <button class="btn btn-outline-secondary" id="topologyManagerResetInline" type="button">
                                        <i class="fa fa-rotate-left"></i> Reset to Saved
                                    </button>
                                    <button class="btn btn-outline-secondary" id="topologyManagerCancelAttachmentEdit" type="button">
                                        <i class="fa fa-ban"></i> Cancel
                                    </button>
                                </div>
                            `
                            : ""}
                        <div class="d-flex flex-wrap gap-2">
                            <button class="btn btn-primary" id="topologyManagerStartMove" type="button" ${legalParentCount === 0 ? "disabled" : ""}>
                                <i class="fa fa-arrows-up-down-left-right"></i> Start Move
                            </button>
                            <button class="btn btn-outline-primary" id="topologyManagerEditAttachmentPreference" type="button" ${hasInspectableAttachmentChoices(meta) && !attachmentEditMode ? "" : "disabled"}>
                                <i class="fa fa-tower-broadcast"></i> Edit Attachment Preference
                            </button>
                        </div>
                    `}

                ${renderManualAttachmentGroupSection(meta)}
            `}
    `;

    bindDetailsInteractions();
}

function buildMapGraph() {
    const meta = selectedMeta();
    if (!meta) {
        return {nodes: [], edges: []};
    }

    const currentPath = currentPathIds(meta.node_id);
    const allowedParents = moveMode ? allowedParentsForSelected() : [];
    const alternatePaths = allowedParents.map((parent) => ({
        parent,
        path: currentPathIds(parent.parent_node_id),
    }));

    const displayedNodes = new Map();
    const displayedEdges = [];

    const rootPathLength = Math.max(
        currentPath.length,
        ...alternatePaths.map((entry) => entry.path.length),
        2,
    );
    const xStep = Math.min(190, Math.max(110, 880 / Math.max(rootPathLength - 1, 1)));
    const startX = 100;

    currentPath.forEach((nodeId, depth) => {
        displayedNodes.set(nodeId, {
            nodeId,
            x: startX + (depth * xStep),
            y: MAP_CENTER_Y,
            depth,
            lane: 0,
            role: "current",
        });
        if (depth > 0) {
            displayedEdges.push({
                from: currentPath[depth - 1],
                to: nodeId,
                kind: "current",
            });
        }
    });

    const selectedDepth = currentPath.length - 1;
    const children = preferredChildPreviewNodes(meta.node_id);
    children.forEach((childId, index) => {
        const rowIndex = index - ((children.length - 1) / 2);
        displayedNodes.set(childId, {
            nodeId: childId,
            x: startX + ((selectedDepth + 1) * xStep),
            y: MAP_CENTER_Y + (rowIndex * CHILD_LANE_STEP),
            depth: selectedDepth + 1,
            lane: rowIndex,
            role: "child",
        });
        displayedEdges.push({
            from: meta.node_id,
            to: childId,
            kind: "branch",
        });
    });

    const alternateLanes = [];
    for (let i = 0; i < alternatePaths.length; i += 1) {
        const offset = Math.floor(i / 2) + 1;
        alternateLanes.push(i % 2 === 0 ? -offset : offset);
    }

    const activeParentId = moveMode
        ? (dragActive && dragHoverParentId ? dragHoverParentId : proposedParentId)
        : null;

    alternatePaths.forEach((entry, index) => {
        const lane = alternateLanes[index];
        entry.path.forEach((nodeId, depth) => {
            if (displayedNodes.has(nodeId)) {
                return;
            }
            displayedNodes.set(nodeId, {
                nodeId,
                x: startX + (depth * xStep),
                y: MAP_CENTER_Y + (lane * LANE_STEP),
                depth,
                lane,
                role: "alternate",
            });
        });
        for (let depth = 1; depth < entry.path.length; depth += 1) {
            displayedEdges.push({
                from: entry.path[depth - 1],
                to: entry.path[depth],
                kind: "candidate_path",
                lane,
            });
        }
        displayedEdges.push({
            from: entry.parent.parent_node_id,
            to: meta.node_id,
            kind: entry.parent.parent_node_id === activeParentId ? "proposed_link" : "candidate_link",
        });
    });

    return {
        nodes: [...displayedNodes.values()],
        edges: displayedEdges,
    };
}

function fitGraphToViewport(graph) {
    if (!graph?.nodes?.length) {
        return graph;
    }

    const minX = Math.min(...graph.nodes.map((node) => node.x - MAP_NODE_HALF_WIDTH));
    const maxX = Math.max(...graph.nodes.map((node) => node.x + MAP_NODE_HALF_WIDTH));
    const minY = Math.min(...graph.nodes.map((node) => node.y - MAP_NODE_HALF_HEIGHT));
    const maxY = Math.max(...graph.nodes.map((node) => node.y + MAP_NODE_HALF_HEIGHT));
    const contentWidth = Math.max(1, maxX - minX);
    const contentHeight = Math.max(1, maxY - minY);
    const centerX = (minX + maxX) / 2;
    const centerY = (minY + maxY) / 2;
    const safeWidth = MAP_VIEWBOX.width - 220;
    const safeHeight = MAP_VIEWBOX.height - 180;
    const xStretch = Math.min(1.22, safeWidth / contentWidth);
    const yStretch = Math.min(1.55, safeHeight / contentHeight);

    return {
        nodes: graph.nodes.map((node) => ({
            ...node,
            x: (MAP_VIEWBOX.width / 2) + ((node.x - centerX) * xStretch),
            y: (MAP_VIEWBOX.height / 2) + ((node.y - centerY) * yStretch),
        })),
        edges: graph.edges,
    };
}

function edgePath(from, to, kind) {
    if (!from || !to) {
        return "";
    }
    if (kind === "candidate_link" || kind === "proposed_link") {
        const midX = (from.x + to.x) / 2;
        const controlOffset = to.y < from.y ? -60 : 60;
        return `M ${from.x} ${from.y} C ${midX} ${from.y + controlOffset}, ${midX} ${to.y - controlOffset}, ${to.x} ${to.y}`;
    }
    return `M ${from.x} ${from.y} L ${to.x} ${to.y}`;
}

function renderMap() {
    const svg = document.getElementById("topologyManagerMap");
    if (!svg) {
        return;
    }
    const meta = selectedMeta();
    if (!meta) {
        resetMapView();
        svg.innerHTML = `
            <rect x="0" y="0" width="${MAP_VIEWBOX.width}" height="${MAP_VIEWBOX.height}" rx="28" fill="transparent"></rect>
            <text x="${MAP_VIEWBOX.width / 2}" y="${MAP_VIEWBOX.height / 2}" text-anchor="middle" fill="rgba(255,255,255,0.7)" font-size="22" font-weight="600">
                Select a branch to render the topology map
            </text>
        `;
        return;
    }

    const graph = fitGraphToViewport(buildMapGraph());
    const nodeById = new Map(graph.nodes.map((node) => [node.nodeId, node]));
    const legalParentIds = new Set(
        moveMode
            ? allowedParentsForSelected().map((parent) => parent.parent_node_id)
            : []
    );

    const edgeHtml = graph.edges.map((edge) => {
        const from = nodeById.get(edge.from);
        const to = nodeById.get(edge.to);
        const path = edgePath(from, to, edge.kind);
        let stroke = "rgba(148, 163, 184, 0.35)";
        let width = 2;
        let dash = "";
        if (edge.kind === "current") {
            stroke = "rgba(59, 130, 246, 0.92)";
            width = 4;
        } else if (edge.kind === "branch") {
            stroke = "rgba(59, 130, 246, 0.45)";
            width = 2.5;
        } else if (edge.kind === "candidate_path") {
            stroke = "rgba(34, 197, 94, 0.35)";
            width = 2;
            dash = "8 8";
        } else if (edge.kind === "candidate_link") {
            stroke = "rgba(34, 197, 94, 0.65)";
            width = 3;
            dash = "10 8";
        } else if (edge.kind === "proposed_link") {
            stroke = "rgba(245, 158, 11, 0.95)";
            width = 4;
            dash = "10 7";
        }
        return `<path d="${path}" fill="none" stroke="${stroke}" stroke-width="${width}" stroke-linecap="round" stroke-dasharray="${dash}"></path>`;
    }).join("");

    const nodeHtml = graph.nodes.map((node) => {
        const topoNode = topologyNodeById.get(node.nodeId) || {};
        const isSelectable = canSelectNodeId(node.nodeId);
        const isEditable = topologyNodeById.has(node.nodeId);
        const label = truncateLabel(displayNameForNodeId(node.nodeId), 22);
        const sublabel = isEditable
            ? nodeKindLabel(node.nodeId)
            : node.nodeId === ROOT_NODE_ID
                ? "Root Context"
                : `${nodeKindLabel(node.nodeId)} Context`;
        const isSelected = node.nodeId === selectedNodeId;
        const isProposed = node.nodeId === proposedParentId;
        const isHoverTarget = dragActive && node.nodeId === dragHoverParentId;
        const isCurrentParent = node.nodeId === meta.current_parent_node_id;
        const isLegalTarget = legalParentIds.has(node.nodeId);
        const hasOverride = !!topoNode.has_override;

        let fill = "rgba(15, 23, 42, 0.90)";
        let stroke = "rgba(148, 163, 184, 0.45)";
        let text = "#f8fafc";
        let badge = "";

        if (isCurrentParent) {
            fill = "rgba(59, 130, 246, 0.20)";
            stroke = "rgba(59, 130, 246, 0.85)";
        }
        if (isLegalTarget) {
            fill = "rgba(22, 163, 74, 0.18)";
            stroke = "rgba(22, 163, 74, 0.85)";
        }
        if (isProposed || isHoverTarget) {
            fill = "rgba(245, 158, 11, 0.22)";
            stroke = "rgba(245, 158, 11, 0.98)";
        }
        if (isSelected) {
            fill = "rgba(59, 130, 246, 0.28)";
            stroke = "rgba(191, 219, 254, 0.95)";
            badge = `<circle cx="78" cy="-34" r="8" fill="#3b82f6"></circle>`;
        } else if (hasOverride) {
            badge = `<circle cx="78" cy="-34" r="7" fill="#f97316"></circle>`;
        }
        if (!isEditable) {
            fill = "rgba(15, 23, 42, 0.62)";
            stroke = "rgba(148, 163, 184, 0.28)";
        }

        const opacity = isSelected
            ? 1
            : isLegalTarget
                ? 0.98
                : !isEditable
                    ? 0.76
                : node.role === "current" || node.role === "child"
                    ? 0.94
                    : 0.52;
        const cursor = !isSelectable
            ? "default"
            : isSelected && meta.can_move && moveMode
                ? "grab"
                : "pointer";

        return `
            <g class="topology-map-node" data-map-node-id="${escapeHtml(node.nodeId)}" transform="translate(${node.x}, ${node.y})" opacity="${opacity}" style="cursor:${cursor};" onclick="window.topologyManagerSelectNode(this.getAttribute('data-map-node-id')); event.stopPropagation();">
                <rect x="-78" y="-36" rx="18" ry="18" width="156" height="72" fill="${fill}" stroke="${stroke}" stroke-width="${isSelected || isProposed ? 3 : 2}"></rect>
                ${badge}
                <text x="0" y="-4" text-anchor="middle" fill="${text}">${escapeHtml(label)}</text>
                <text x="0" y="18" text-anchor="middle" fill="rgba(248,250,252,0.72)" class="topology-map-node-label-muted">${escapeHtml(sublabel)}</text>
            </g>
        `;
    }).join("");

    svg.setAttribute("viewBox", `0 0 ${MAP_VIEWBOX.width} ${MAP_VIEWBOX.height}`);
    const mapCursor = mapPanActive ? "grabbing" : "grab";

    svg.innerHTML = `
        <defs>
            <filter id="topologyNodeGlow" x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="10" result="blur"></feGaussianBlur>
                <feMerge>
                    <feMergeNode in="blur"></feMergeNode>
                    <feMergeNode in="SourceGraphic"></feMergeNode>
                </feMerge>
            </filter>
        </defs>
        <rect x="0" y="0" width="${MAP_VIEWBOX.width}" height="${MAP_VIEWBOX.height}" rx="28" fill="transparent" style="cursor:${mapCursor};"></rect>
        <g id="topologyManagerViewport" transform="${mapViewportTransform()}">
            ${edgeHtml}
            ${nodeHtml}
        </g>
    `;

    svg.style.cursor = mapCursor;
    svg.onpointerdown = (event) => {
        if (event.target.closest("[data-map-node-id]")) {
            return;
        }
        mapPanActive = true;
        mapPanMoved = false;
        mapPanStart = {x: event.clientX, y: event.clientY};
        mapPanStartView = {...mapView};
        svg.style.cursor = "grabbing";
        if (typeof svg.setPointerCapture === "function") {
            try {
                svg.setPointerCapture(event.pointerId);
            } catch (_) {
                // Ignore pointer-capture failures on unsupported browsers.
            }
        }
    };
    svg.onwheel = (event) => {
        event.preventDefault();
        const anchor = mapPointFromClient(svg, event.clientX, event.clientY);
        const direction = event.deltaY > 0 ? -1 : 1;
        const nextScale = mapView.scale * (1 + (direction * MAP_ZOOM_STEP));
        setMapZoomAroundPoint(nextScale, anchor);
        renderMap();
    };
    svg.ondblclick = (event) => {
        if (event.target.closest("[data-map-node-id]")) {
            return;
        }
        resetMapView();
        renderMap();
    };

    svg.querySelectorAll("[data-map-node-id]").forEach((nodeEl) => {
        nodeEl.addEventListener("pointerdown", (event) => {
            const nodeId = nodeEl.getAttribute("data-map-node-id");
            if (!nodeId || nodeId !== selectedNodeId || !meta.can_move || !moveMode) {
                return;
            }
            dragActive = true;
            dragMoved = false;
            dragHoverParentId = null;
            dragStart = {x: event.clientX, y: event.clientY};
            nodeEl.style.cursor = "grabbing";
        });
        nodeEl.addEventListener("click", () => {
            if (suppressNextMapClick) {
                suppressNextMapClick = false;
                return;
            }
            const nodeId = nodeEl.getAttribute("data-map-node-id");
            if (!nodeId) return;
            setSelectedNode(nodeId);
        });
    });
}

function updateDragHoverFromPointer(event) {
    if (mapPanActive) {
        if (!mapPanStart || !mapPanStartView) {
            return;
        }
        const dx = event.clientX - mapPanStart.x;
        const dy = event.clientY - mapPanStart.y;
        if ((dx * dx) + (dy * dy) > MAP_PAN_THRESHOLD_SQ) {
            mapPanMoved = true;
        }
        mapView = {
            ...mapPanStartView,
            x: mapPanStartView.x + dx,
            y: mapPanStartView.y + dy,
        };
        renderMap();
        return;
    }

    if (!dragActive) {
        return;
    }
    if (dragStart) {
        const dx = event.clientX - dragStart.x;
        const dy = event.clientY - dragStart.y;
        if ((dx * dx) + (dy * dy) > MAP_PAN_THRESHOLD_SQ) {
            dragMoved = true;
        }
    }
    const hoveredEl = document.elementFromPoint(event.clientX, event.clientY)?.closest("[data-map-node-id]");
    const hoveredId = hoveredEl?.getAttribute("data-map-node-id") || null;
    const legalParentIds = new Set(allowedParentsForSelected().map((parent) => parent.parent_node_id));
    const nextHover = hoveredId && hoveredId !== selectedNodeId && legalParentIds.has(hoveredId)
        ? hoveredId
        : null;
    if (nextHover !== dragHoverParentId) {
        dragHoverParentId = nextHover;
        renderMap();
    }
}

function finishMapDrag() {
    if (mapPanActive) {
        const didPan = mapPanMoved;
        mapPanActive = false;
        mapPanMoved = false;
        mapPanStart = null;
        mapPanStartView = null;
        if (didPan) {
            suppressNextMapClick = true;
        }
        renderMap();
        return;
    }

    if (!dragActive) {
        return;
    }
    const dropTargetId = dragMoved ? dragHoverParentId : null;
    const didDrag = dragMoved;
    dragActive = false;
    dragMoved = false;
    dragStart = null;
    dragHoverParentId = null;
    if (didDrag) {
        suppressNextMapClick = true;
    }
    if (dropTargetId) {
        setProposedParent(dropTargetId, {autoCommit: true});
        return;
    }
    renderMap();
}

async function setProbePolicy(pairId, enabled) {
    try {
        const response = await sendWsRequest("SetTopologyManagerProbePolicyResult", {
            SetTopologyManagerProbePolicy: {
                update: {
                    attachment_pair_id: pairId,
                    enabled,
                },
            },
        });
        indexTopologyState(response.data);
        lastFlash = enabled
            ? "Attachment probing enabled."
            : "Attachment probing disabled.";
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to update attachment probe policy."], null);
    }
}

function syncManualAttachmentDraftFromInputs() {
    if (!manualAttachmentEditMode) {
        return;
    }
    const nextRows = manualAttachmentDraftRows.map((row, index) => {
        const nextRow = {...row};
        document.querySelectorAll(`[data-manual-row-index="${index}"]`).forEach((input) => {
            const field = input.getAttribute("data-manual-field");
            if (!field) {
                return;
            }
            nextRow[field] = input.type === "checkbox" ? input.checked : input.value;
        });
        return nextRow;
    });
    manualAttachmentDraftRows = nextRows;
}

function enterManualAttachmentEditMode() {
    const meta = selectedMeta();
    const parent = manualAttachmentParentMeta(meta);
    if (!meta?.can_move || !parent) {
        return;
    }
    manualAttachmentEditMode = true;
    manualAttachmentDraftRows = cloneManualAttachmentRows(parent);
    if (manualAttachmentDraftRows.length === 0) {
        manualAttachmentDraftRows = [blankManualAttachmentRow(), blankManualAttachmentRow()];
    }
    renderEverything();
}

function cancelManualAttachmentEditMode() {
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    renderEverything();
}

async function saveManualAttachmentGroup() {
    const meta = selectedMeta();
    const parent = manualAttachmentParentMeta(meta);
    if (!meta || !parent) {
        return;
    }
    syncManualAttachmentDraftFromInputs();
    try {
        const response = await sendWsRequest("SetTopologyManagerManualAttachmentGroupResult", {
            SetTopologyManagerManualAttachmentGroup: {
                update: {
                    child_node_id: meta.node_id,
                    parent_node_id: parent.parent_node_id,
                    attachments: manualAttachmentDraftRows.map((row) => ({
                        attachment_id: (row.attachment_id || "").trim(),
                        attachment_name: (row.attachment_name || "").trim(),
                        capacity_mbps: Number.parseInt(String(row.capacity_mbps || ""), 10) || 0,
                        local_probe_ip: (row.local_probe_ip || "").trim(),
                        remote_probe_ip: (row.remote_probe_ip || "").trim(),
                        probe_enabled: !!row.probe_enabled,
                    })),
                },
            },
        });
        indexTopologyState(response.data);
        manualAttachmentEditMode = false;
        manualAttachmentDraftRows = [];
        lastFlash = "Manual attachment group saved.";
        if (selectedNodeId && topologyNodeById.has(selectedNodeId)) {
            initializeProposalFromSaved(selectedMeta());
        }
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to save manual attachment group."], null);
    }
}

async function clearManualAttachmentGroup() {
    const meta = selectedMeta();
    const parent = manualAttachmentParentMeta(meta);
    if (!meta || !parent) {
        return;
    }
    try {
        const response = await sendWsRequest("ClearTopologyManagerManualAttachmentGroupResult", {
            ClearTopologyManagerManualAttachmentGroup: {
                clear: {
                    child_node_id: meta.node_id,
                    parent_node_id: parent.parent_node_id,
                },
            },
        });
        indexTopologyState(response.data);
        manualAttachmentEditMode = false;
        manualAttachmentDraftRows = [];
        lastFlash = "Manual attachment group cleared.";
        if (selectedNodeId && topologyNodeById.has(selectedNodeId)) {
            initializeProposalFromSaved(selectedMeta());
        }
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to clear manual attachment group."], null);
    }
}

function bindDetailsInteractions() {
    document.getElementById("topologyManagerStartMove")?.addEventListener("click", () => {
        enterMoveMode();
    });
    document.getElementById("topologyManagerCancelMove")?.addEventListener("click", () => {
        cancelMoveMode();
    });
    document.getElementById("topologyManagerEditAttachmentPreference")?.addEventListener("click", () => {
        enterAttachmentEditMode();
    });
    document.getElementById("topologyManagerCancelAttachmentEdit")?.addEventListener("click", () => {
        cancelAttachmentEditMode();
    });
    document.getElementById("topologyManagerEditManualAttachments")?.addEventListener("click", () => {
        enterManualAttachmentEditMode();
    });
    document.getElementById("topologyManagerCancelManualAttachments")?.addEventListener("click", () => {
        cancelManualAttachmentEditMode();
    });
    document.getElementById("topologyManagerAddManualAttachment")?.addEventListener("click", () => {
        syncManualAttachmentDraftFromInputs();
        if (manualAttachmentDraftRows.length >= 8) {
            return;
        }
        manualAttachmentDraftRows = [...manualAttachmentDraftRows, blankManualAttachmentRow()];
        renderEverything();
    });
    document.getElementById("topologyManagerSaveManualAttachments")?.addEventListener("click", () => {
        saveManualAttachmentGroup();
    });
    document.getElementById("topologyManagerClearManualAttachments")?.addEventListener("click", () => {
        clearManualAttachmentGroup();
    });
    document.querySelectorAll("[data-parent-target-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const parentId = button.getAttribute("data-parent-target-id");
            if (parentId) {
                setProposedParent(parentId, {autoCommit: true});
            }
        });
    });

    document.querySelectorAll('input[name="topologyAttachmentMode"]').forEach((radio) => {
        radio.addEventListener("change", () => {
            proposedMode = radio.value;
            if (proposedMode === "auto") {
                proposedAttachmentIds = [];
            }
            renderEverything();
        });
    });

    document.querySelectorAll("[data-add-attachment-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const attachmentId = button.getAttribute("data-add-attachment-id");
            if (!attachmentId || proposedAttachmentIds.includes(attachmentId)) {
                return;
            }
            proposedMode = "preferred_order";
            proposedAttachmentIds = [...proposedAttachmentIds, attachmentId];
            renderEverything();
        });
    });

    document.querySelectorAll("[data-remove-attachment-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const attachmentId = button.getAttribute("data-remove-attachment-id");
            proposedAttachmentIds = proposedAttachmentIds.filter((id) => id !== attachmentId);
            renderEverything();
        });
    });

    document.querySelectorAll("[data-rank-up-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const attachmentId = button.getAttribute("data-rank-up-id");
            const index = proposedAttachmentIds.indexOf(attachmentId);
            if (index <= 0) return;
            const next = [...proposedAttachmentIds];
            [next[index - 1], next[index]] = [next[index], next[index - 1]];
            proposedAttachmentIds = next;
            renderEverything();
        });
    });

    document.querySelectorAll("[data-rank-down-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const attachmentId = button.getAttribute("data-rank-down-id");
            const index = proposedAttachmentIds.indexOf(attachmentId);
            if (index < 0 || index >= proposedAttachmentIds.length - 1) return;
            const next = [...proposedAttachmentIds];
            [next[index], next[index + 1]] = [next[index + 1], next[index]];
            proposedAttachmentIds = next;
            renderEverything();
        });
    });
    document.querySelectorAll("[data-probe-policy-pair-id]").forEach((button) => {
        button.addEventListener("click", async () => {
            const pairId = button.getAttribute("data-probe-policy-pair-id");
            const enabled = button.getAttribute("data-probe-policy-enabled") === "true";
            if (!pairId) {
                return;
            }
            await setProbePolicy(pairId, enabled);
        });
    });
    document.querySelectorAll("[data-remove-manual-row]").forEach((button) => {
        button.addEventListener("click", () => {
            syncManualAttachmentDraftFromInputs();
            const index = Number.parseInt(button.getAttribute("data-remove-manual-row") || "", 10);
            if (!Number.isInteger(index) || manualAttachmentDraftRows.length <= 2) {
                return;
            }
            manualAttachmentDraftRows = manualAttachmentDraftRows.filter((_, rowIndex) => rowIndex !== index);
            renderEverything();
        });
    });

    document.getElementById("topologyManagerSave")?.addEventListener("click", () => {
        saveProposal();
    });
    document.getElementById("topologyManagerResetInline")?.addEventListener("click", () => {
        initializeProposalFromSaved();
        renderEverything();
    });
}

function setSelectedNode(nodeId) {
    if (!canSelectNodeId(nodeId)) {
        return;
    }
    clearSearchSuggestions();
    selectedNodeId = nodeId;
    moveMode = false;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    resetMapView();
    initializeProposalFromSaved();
    lastFlash = null;
    renderEverything();
}

function enterMoveMode() {
    const meta = selectedMeta();
    if (!meta?.can_move) {
        return;
    }
    if (allowedParentsForSelected().length === 0) {
        lastFlash = "No legal parent targets are currently available for this branch.";
        renderEverything();
        return;
    }
    moveMode = true;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    initializeProposalFromSaved(meta);
    renderEverything();
}

function cancelMoveMode() {
    moveMode = false;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    initializeProposalFromSaved();
    renderEverything();
}

function enterAttachmentEditMode() {
    const meta = selectedMeta();
    const parent = attachmentEditParentMeta(meta);
    if (!meta?.can_move || !parent) {
        return;
    }
    moveMode = false;
    attachmentEditMode = true;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    proposedParentId = parent.parent_node_id;
    if (meta.has_override && meta.override_parent_node_id === parent.parent_node_id) {
        proposedMode = meta.override_mode || "auto";
        proposedAttachmentIds = [...(meta.override_attachment_preference_ids || [])];
    } else {
        proposedMode = "auto";
        proposedAttachmentIds = [];
    }
    renderEverything();
}

function cancelAttachmentEditMode() {
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    initializeProposalFromSaved();
    renderEverything();
}

function canAutoCommitParent(parent) {
    return explicitAttachmentOptions(parent).length <= 1;
}

async function setProposedParent(parentId, options = {}) {
    if (!parentId) {
        proposedParentId = null;
        proposedMode = "auto";
        proposedAttachmentIds = [];
        renderEverything();
        return;
    }

    const parent = allowedParentsForSelected().find((entry) => entry.parent_node_id === parentId);
    if (!parent) {
        return;
    }
    proposedParentId = parentId;

    const meta = selectedMeta();
    if (meta?.has_override && meta.override_parent_node_id === parentId) {
        proposedMode = meta.override_mode || "auto";
        proposedAttachmentIds = [...(meta.override_attachment_preference_ids || [])];
    } else {
        proposedMode = "auto";
        proposedAttachmentIds = [];
        const explicitOptions = explicitAttachmentOptions(parent);
        if (explicitOptions.length === 1) {
            proposedMode = "preferred_order";
            proposedAttachmentIds = [explicitOptions[0].attachment_id];
        }
    }

    if (options.autoCommit && canAutoCommitParent(parent) && proposalIsValid() && !proposalMatchesSavedOverride()) {
        renderEverything();
        await saveProposal();
        return;
    }
    renderEverything();
}

async function saveProposal() {
    if (!proposalIsValid() || !selectedNodeId || !proposedParentId) {
        return;
    }

    try {
        const response = await sendWsRequest("SetTopologyManagerOverrideResult", {
            SetTopologyManagerOverride: {
                update: {
                    child_node_id: selectedNodeId,
                    parent_node_id: proposedParentId,
                    mode: proposedMode,
                    attachment_preference_ids: proposedMode === "preferred_order" ? proposedAttachmentIds : [],
                },
            },
        });
        indexTopologyState(response.data);
        moveMode = false;
        attachmentEditMode = false;
        manualAttachmentEditMode = false;
        manualAttachmentDraftRows = [];
        lastFlash = "Topology override saved. It will be applied on the next scheduler/integration run.";
        if (selectedNodeId && topologyNodeById.has(selectedNodeId)) {
            initializeProposalFromSaved(selectedMeta());
        }
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to save topology override."], null);
    }
}

async function clearSavedOverride() {
    const meta = selectedMeta();
    if (!meta) {
        return;
    }
    try {
        const response = await sendWsRequest("ClearTopologyManagerOverrideResult", {
            ClearTopologyManagerOverride: {
                clear: {
                    child_node_id: meta.node_id,
                },
            },
        });
        indexTopologyState(response.data);
        moveMode = false;
        attachmentEditMode = false;
        manualAttachmentEditMode = false;
        manualAttachmentDraftRows = [];
        lastFlash = "Saved topology override cleared.";
        if (selectedNodeId && topologyNodeById.has(selectedNodeId)) {
            initializeProposalFromSaved(selectedMeta());
        }
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to clear saved topology override."], null);
    }
}

function renderEverything(warnings = null) {
    updateHeader();
    renderModeBanner();
    renderMap();
    renderHierarchyPanel();
    renderDetailsPanel();
    showWarnings(
        warnings || topologyManagerState?.global_warnings || [],
        lastFlash,
    );
}

function bindPageControls() {
    document.getElementById("topologyManagerFitMap")?.addEventListener("click", () => {
        resetMapView();
        renderMap();
    });
    document.getElementById("topologyManagerResetProposal")?.addEventListener("click", () => {
        initializeProposalFromSaved();
        renderEverything();
    });
    document.getElementById("topologyManagerClearSaved")?.addEventListener("click", () => {
        clearSavedOverride();
    });

    const searchInput = document.getElementById("topologyManagerNodeSearch");
    const searchSuggestions = document.getElementById("topologyManagerSearchSuggestions");
    searchInput?.addEventListener("input", () => {
        renderSearchSuggestions(searchInput.value || "");
    });
    searchInput?.addEventListener("focus", () => {
        if ((searchInput.value || "").trim()) {
            renderSearchSuggestions(searchInput.value || "");
        }
    });
    searchInput?.addEventListener("keydown", (event) => {
        if (event.key === "ArrowDown") {
            if (searchSuggestionNodeIds.length === 0) {
                renderSearchSuggestions(searchInput.value || "");
            }
            if (searchSuggestionNodeIds.length === 0) {
                return;
            }
            event.preventDefault();
            searchSuggestionIndex = Math.min(
                searchSuggestionNodeIds.length - 1,
                searchSuggestionIndex + 1,
            );
            updateSearchSuggestionHighlight();
            return;
        }
        if (event.key === "ArrowUp") {
            if (searchSuggestionNodeIds.length === 0) {
                return;
            }
            event.preventDefault();
            searchSuggestionIndex = Math.max(0, searchSuggestionIndex - 1);
            updateSearchSuggestionHighlight();
            return;
        }
        if (event.key === "Escape") {
            clearSearchSuggestions();
            return;
        }
        if (event.key !== "Enter") {
            return;
        }
        event.preventDefault();
        if (searchSuggestionIndex >= 0 && searchSuggestionIndex < searchSuggestionNodeIds.length) {
            selectSearchSuggestion(searchSuggestionNodeIds[searchSuggestionIndex]);
            return;
        }
        const query = (searchInput.value || "").trim().toLowerCase();
        if (!query) {
            return;
        }
        const matches = rankedSearchMatches(query);
        if (matches.length > 0) {
            selectSearchSuggestion(matches[0].node_id);
            return;
        }
        lastFlash = `No topology node matched "${query}".`;
        clearSearchSuggestions();
        renderEverything();
    });
    searchInput?.addEventListener("blur", () => {
        window.setTimeout(() => {
            clearSearchSuggestions();
        }, 120);
    });
    searchSuggestions?.addEventListener("mousedown", (event) => {
        const button = event.target.closest("[data-search-node-id]");
        if (!button) {
            return;
        }
        event.preventDefault();
        const nodeId = button.getAttribute("data-search-node-id");
        selectSearchSuggestion(nodeId);
    });
}

document.addEventListener("pointermove", (event) => {
    updateDragHoverFromPointer(event);
});

document.addEventListener("pointerup", () => {
    finishMapDrag();
});

document.addEventListener("pointercancel", () => {
    finishMapDrag();
});

async function loadPage() {
    try {
        const [treeResponse, stateResponse] = await Promise.all([
            sendWsRequest("NetworkTreeLite", {NetworkTreeLite: {}}),
            sendWsRequest("GetTopologyManagerState", {GetTopologyManagerState: {}}),
        ]);

        indexTopologyState(stateResponse.data);
        indexNetworkTree(treeResponse.data || []);

        if (!canSelectNodeId(selectedNodeId)) {
            selectedNodeId = pickDefaultNodeId();
        }
        initializeProposalFromSaved();
        lastFlash = null;
        renderEverything(stateResponse.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to load topology manager state."], null);
        const map = document.getElementById("topologyManagerMap");
        if (map) {
            map.innerHTML = `
                <text x="${MAP_VIEWBOX.width / 2}" y="${MAP_VIEWBOX.height / 2}" text-anchor="middle" fill="rgba(255,255,255,0.7)" font-size="22" font-weight="600">
                    Unable to load topology manager
                </text>
            `;
        }
    }
}

bindPageControls();
window.topologyManagerSelectNode = selectNodeFromMap;
listenOnce("join", () => {
    loadPage();
});
loadPage();
