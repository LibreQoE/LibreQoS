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
const MAX_VISIBLE_UPSTREAM_PREVIEW_NODES = 2;
const MAX_CHILD_PREVIEW_NODES = 6;
const MAX_SEARCH_SUGGESTIONS = 8;
const SELECTED_NODE_URL_PARAM = "node_id";
const TOPOLOGY_STATE_REFRESH_INTERVAL_MS = 4000;
const TOPOLOGY_STATE_REFRESH_VISIBLE_DELAY_MS = 250;
const TOPOLOGY_STATE_REFRESH_EDIT_DEFER_MS = 1200;

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
let saveProposalInFlight = false;
let manualAttachmentEditMode = false;
let manualAttachmentDraftRows = [];
let attachmentRateEditParentId = null;
let attachmentRateEditAttachmentId = null;
let attachmentRateDraftDownloadMbps = "";
let attachmentRateDraftUploadMbps = "";
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
let topologyStateRefreshTimer = null;
let topologyStateRefreshInFlight = false;

function clearAttachmentRateEditState() {
    attachmentRateEditParentId = null;
    attachmentRateEditAttachmentId = null;
    attachmentRateDraftDownloadMbps = "";
    attachmentRateDraftUploadMbps = "";
}

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

function selectedNodeIdFromUrl() {
    try {
        const params = new URLSearchParams(window.location.search || "");
        const value = (params.get(SELECTED_NODE_URL_PARAM) || "").trim();
        return value || null;
    } catch (error) {
        return null;
    }
}

function preferredInitialNodeId() {
    return selectedNodeIdFromUrl() || ROOT_NODE_ID;
}

function urlWithSelectedNodeId(nodeId) {
    const url = new URL(window.location.href);
    if (nodeId) {
        url.searchParams.set(SELECTED_NODE_URL_PARAM, nodeId);
    } else {
        url.searchParams.delete(SELECTED_NODE_URL_PARAM);
    }
    return `${url.pathname}${url.search}${url.hash}`;
}

function syncSelectedNodeUrl(nodeId, mode = "replace") {
    const nextUrl = urlWithSelectedNodeId(nodeId);
    const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (nextUrl === currentUrl) {
        return;
    }
    const method = mode === "push" ? "pushState" : "replaceState";
    history[method](null, "", nextUrl);
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
        || "Unknown node";
}

function treePageHrefForNodeId(nodeId) {
    const normalizedId = (nodeId || "").trim();
    if (!normalizedId || normalizedId === ROOT_NODE_ID) {
        return null;
    }
    const params = new URLSearchParams();
    params.set("parent", "0");
    params.set("nodeId", normalizedId);
    return `/tree.html?${params.toString()}`;
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
        rememberNodeLabel(node.effective_parent_node_id, node.effective_parent_node_name);
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

function hasUnsavedTopologyEdits() {
    return moveMode || attachmentEditMode || manualAttachmentEditMode || !!attachmentRateEditAttachmentId;
}

function reconcileSelectedNodeId(preferredNodeId = null) {
    const nextSelectedNodeId = canSelectNodeId(preferredNodeId)
        ? preferredNodeId
        : (canSelectNodeId(selectedNodeId) ? selectedNodeId : pickDefaultNodeId());
    const selectionChanged = nextSelectedNodeId !== selectedNodeId;
    if (selectionChanged) {
        selectedNodeId = nextSelectedNodeId;
        moveMode = false;
        attachmentEditMode = false;
        manualAttachmentEditMode = false;
        manualAttachmentDraftRows = [];
        clearAttachmentRateEditState();
        resetMapView();
    }
    syncSelectedNodeUrl(selectedNodeId, "replace");
    return selectionChanged;
}

function applyTopologyManagerState(data, options = {}) {
    indexTopologyState(data);
    const selectionChanged = reconcileSelectedNodeId(options.preferredNodeId || null);
    if (!options.preserveEdits || selectionChanged) {
        initializeProposalFromSaved();
    }
    if (attachmentRateEditAttachmentId && !attachmentRateEditOption()) {
        clearAttachmentRateEditState();
    }
    if (Object.prototype.hasOwnProperty.call(options, "flash")) {
        lastFlash = options.flash;
    }
    renderEverything(options.warnings || data?.global_warnings || []);
}

function clearTopologyStateRefreshTimer() {
    if (topologyStateRefreshTimer !== null) {
        window.clearTimeout(topologyStateRefreshTimer);
        topologyStateRefreshTimer = null;
    }
}

function hasFocusedTopologyEditorInput() {
    const active = document.activeElement;
    const details = document.getElementById("topologyManagerDetails");
    if (!active || !details || !details.contains(active)) {
        return false;
    }
    const tagName = (active.tagName || "").toUpperCase();
    return tagName === "INPUT" || tagName === "TEXTAREA" || tagName === "SELECT";
}

function scheduleTopologyStateRefresh(delayMs = TOPOLOGY_STATE_REFRESH_INTERVAL_MS) {
    clearTopologyStateRefreshTimer();
    topologyStateRefreshTimer = window.setTimeout(() => {
        refreshTopologyManagerState();
    }, delayMs);
}

async function refreshTopologyManagerState(options = {}) {
    if (topologyStateRefreshInFlight) {
        return;
    }
    if (!options.force && document.hidden) {
        scheduleTopologyStateRefresh();
        return;
    }
    if (!options.force && hasFocusedTopologyEditorInput()) {
        scheduleTopologyStateRefresh(TOPOLOGY_STATE_REFRESH_EDIT_DEFER_MS);
        return;
    }
    topologyStateRefreshInFlight = true;
    try {
        const response = await sendWsRequest("GetTopologyManagerState", {GetTopologyManagerState: {}});
        applyTopologyManagerState(response.data, {
            preserveEdits: hasUnsavedTopologyEdits(),
        });
    } catch (error) {
        if (options.reportErrors) {
            showWarnings([error?.message || "Unable to refresh topology manager state."], null);
        }
    } finally {
        topologyStateRefreshInFlight = false;
        scheduleTopologyStateRefresh();
    }
}

function synthesizeContextMeta(nodeId) {
    if (nodeId === ROOT_NODE_ID) {
        return {
            node_id: ROOT_NODE_ID,
            node_name: "Root",
            current_parent_node_id: null,
            current_parent_node_name: null,
            effective_parent_node_id: null,
            effective_parent_node_name: null,
            can_move: false,
            allowed_parents: [],
            has_override: false,
            override_parent_node_id: null,
            override_parent_node_name: null,
            override_mode: "auto",
            override_attachment_preference_ids: [],
            override_attachment_preference_names: [],
            override_live: false,
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
        effective_parent_node_id: parent?.id || null,
        effective_parent_node_name: parent?.name || null,
        can_move: false,
        allowed_parents: [],
        has_override: false,
        override_parent_node_id: null,
        override_parent_node_name: null,
        override_mode: "auto",
        override_attachment_preference_ids: [],
        override_attachment_preference_names: [],
        override_live: false,
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
        effective_parent_node_id: ROOT_NODE_ID,
        effective_parent_node_name: "Root",
        can_move: false,
        allowed_parents: [],
        has_override: false,
        override_parent_node_id: null,
        override_parent_node_name: null,
        override_mode: "auto",
        override_attachment_preference_ids: [],
        override_attachment_preference_names: [],
        override_live: false,
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
    return pathIdsForNode(nodeId, "current_parent_node_id");
}

function effectivePathIds(nodeId) {
    return pathIdsForNode(nodeId, "effective_parent_node_id");
}

function pathIdsForNode(nodeId, parentField) {
    const path = [];
    const seen = new Set();
    let cursor = nodeId;
    while (cursor && !seen.has(cursor)) {
        seen.add(cursor);
        const node = topologyNodeById.get(cursor);
        path.unshift(cursor);
        cursor = node?.[parentField] || treeParentNodeId(cursor) || null;
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

function truncatedPreviewPath(pathIds, markerKey) {
    if (!Array.isArray(pathIds) || pathIds.length === 0) {
        return [];
    }
    const visibleCount = MAX_VISIBLE_UPSTREAM_PREVIEW_NODES + 1;
    if (pathIds.length <= visibleCount) {
        return pathIds.map((nodeId) => ({nodeId}));
    }

    const tail = pathIds.slice(-visibleCount);
    const omittedCount = pathIds.length - tail.length;
    return [
        {
            nodeId: `__topology_preview_stub__:${markerKey}`,
            label: "…",
            sublabel: `${omittedCount} earlier ${omittedCount === 1 ? "node" : "nodes"}`,
            isSynthetic: true,
        },
        ...tail.map((nodeId) => ({nodeId})),
    ];
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

function formatBandwidthValue(value) {
    const numeric = Number.parseInt(String(value ?? ""), 10);
    return Number.isFinite(numeric) && numeric > 0 ? `${numeric} Mbps` : null;
}

function formatAttachmentBandwidth(option) {
    const download = Number.parseInt(String(option?.download_bandwidth_mbps ?? ""), 10);
    const upload = Number.parseInt(String(option?.upload_bandwidth_mbps ?? ""), 10);
    if (Number.isFinite(download) && download > 0 && Number.isFinite(upload) && upload > 0) {
        return download === upload
            ? `${download} Mbps`
            : `${download} down / ${upload} up Mbps`;
    }
    return formatBandwidthValue(option?.capacity_mbps);
}

function attachmentRateSourceLabel(option) {
    switch (option?.rate_source) {
    case "dynamic_integration":
        return "Dynamic integration rate";
    case "manual":
        return "Manual attachment rate";
    case "static":
        return "Static attachment rate";
    default:
        return null;
    }
}

function attachmentRoleLabel(option) {
    switch (option?.attachment_role) {
    case "ptp_backhaul":
        return "PtP Backhaul";
    case "ptmp_uplink":
        return "PtMP Uplink";
    case "wired_uplink":
        return "Wired Uplink";
    case "manual":
        return "Manual";
    default:
        return null;
    }
}

function isEditingAttachmentRate(parentMeta, option) {
    return !!parentMeta
        && !!option
        && attachmentRateEditParentId === parentMeta.parent_node_id
        && attachmentRateEditAttachmentId === option.attachment_id;
}

function attachmentRateEditOption() {
    const meta = selectedMeta();
    if (!meta || !attachmentRateEditParentId || !attachmentRateEditAttachmentId) {
        return null;
    }
    const parent = allowedParentsForSelected().find((entry) => entry.parent_node_id === attachmentRateEditParentId);
    if (!parent) {
        return null;
    }
    const option = explicitAttachmentOptions(parent).find((entry) => entry.attachment_id === attachmentRateEditAttachmentId);
    if (!option) {
        return null;
    }
    return {parent, option};
}

function startAttachmentRateEdit(parentNodeId, attachmentId) {
    const parent = allowedParentsForSelected().find((entry) => entry.parent_node_id === parentNodeId);
    const option = explicitAttachmentOptions(parent).find((entry) => entry.attachment_id === attachmentId);
    if (!topologyManagerState?.writable || !option?.can_override_rate) {
        return;
    }
    moveMode = false;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    attachmentRateEditParentId = parentNodeId;
    attachmentRateEditAttachmentId = attachmentId;
    attachmentRateDraftDownloadMbps = String(option.download_bandwidth_mbps ?? option.capacity_mbps ?? "");
    attachmentRateDraftUploadMbps = String(option.upload_bandwidth_mbps ?? option.capacity_mbps ?? "");
    renderEverything();
}

function cancelAttachmentRateEditMode() {
    clearAttachmentRateEditState();
    renderEverything();
}

function syncAttachmentRateDraftFromInputs() {
    if (!attachmentRateEditAttachmentId) {
        return;
    }
    const downloadInput = document.getElementById("topologyManagerAttachmentRateDownload");
    const uploadInput = document.getElementById("topologyManagerAttachmentRateUpload");
    if (downloadInput) {
        attachmentRateDraftDownloadMbps = downloadInput.value || "";
    }
    if (uploadInput) {
        attachmentRateDraftUploadMbps = uploadInput.value || "";
    }
}

function attachmentRateSaveDisabled() {
    const download = Number.parseInt(String(attachmentRateDraftDownloadMbps || ""), 10);
    const upload = Number.parseInt(String(attachmentRateDraftUploadMbps || ""), 10);
    return !Number.isFinite(download) || download <= 0 || !Number.isFinite(upload) || upload <= 0;
}

function refreshAttachmentRateSaveButton() {
    const button = document.getElementById("topologyManagerSaveAttachmentRate");
    if (button) {
        button.disabled = attachmentRateSaveDisabled();
    }
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

function attachmentProbeBadge(option) {
    return option?.probe_enabled
        ? "<span class='badge text-bg-info'>Probe On</span>"
        : "<span class='badge text-bg-secondary'>Probe Off</span>";
}

function attachmentSelectionBadges(parentMeta, option) {
    const meta = selectedMeta();
    const badges = [];
    const isUsingNow = !!option?.effective_selected;
    const isPreferred = !!meta
        && parentMeta?.parent_node_id === attachmentEditParentId(meta)
        && (
            (meta.override_attachment_preference_ids || []).includes(option?.attachment_id)
            || meta.preferred_attachment_name === option?.attachment_name
        );

    if (isUsingNow && isPreferred) {
        badges.push("<span class='badge text-bg-primary'>Using Now / Preferred</span>");
        return badges.join("");
    }
    if (isUsingNow) {
        badges.push("<span class='badge text-bg-primary'>Using Now</span>");
    }
    if (isPreferred) {
        badges.push("<span class='badge text-bg-success'>Preferred</span>");
    }
    return badges.join("");
}

function attachmentStatusLine(option) {
    const parts = [];
    const bandwidth = formatAttachmentBandwidth(option);
    if (bandwidth) {
        parts.push(escapeHtml(bandwidth));
    }
    parts.push(option?.probe_enabled ? "Probe On" : "Probe Off");
    if (option?.has_rate_override) {
        parts.push("Attachment Rate");
    }
    const rateSource = attachmentRateSourceLabel(option);
    if (rateSource) {
        parts.push(escapeHtml(rateSource));
    }
    return parts.join(" • ");
}

function attachmentReasonNote(option) {
    const rows = [];
    if (option?.health_reason) {
        rows.push(escapeHtml(option.health_reason));
    }
    if (option?.suppressed_until_unix) {
        rows.push(`Suppressed until ${escapeHtml(formatUnixTimestamp(option.suppressed_until_unix))}`);
    }
    if (option?.transport_cap_reason) {
        const capText = formatBandwidthValue(option.transport_cap_mbps);
        rows.push(capText
            ? `Transport cap ${escapeHtml(capText)}: ${escapeHtml(option.transport_cap_reason)}`
            : escapeHtml(option.transport_cap_reason));
    }
    if (option?.local_probe_ip || option?.remote_probe_ip) {
        rows.push(`${escapeHtml(option.local_probe_ip || "?")} ↔ ${escapeHtml(option.remote_probe_ip || "?")}`);
    }
    return rows.join(" · ");
}

function attachmentHealthDetails(option) {
    const rows = [];
    if (option?.health_reason) {
        rows.push(escapeHtml(option.health_reason));
    }
    if (option?.suppressed_until_unix) {
        rows.push(`Suppressed until ${escapeHtml(formatUnixTimestamp(option.suppressed_until_unix))}`);
    }
    const bandwidth = formatAttachmentBandwidth(option);
    if (bandwidth) {
        rows.push(escapeHtml(bandwidth));
    }
    const rateSource = attachmentRateSourceLabel(option);
    if (rateSource) {
        rows.push(escapeHtml(rateSource));
    }
    if (option?.has_rate_override) {
        rows.push("Attachment rate override active");
    }
    if (option?.transport_cap_reason) {
        const capText = formatBandwidthValue(option.transport_cap_mbps);
        rows.push(capText
            ? `Transport cap ${escapeHtml(capText)}: ${escapeHtml(option.transport_cap_reason)}`
            : escapeHtml(option.transport_cap_reason));
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
        container.innerHTML = "<span class='topology-manager-inline-note'><i class='fa fa-circle-info'></i>Select a branch to inspect or edit it.</span>";
        return;
    }

    if (!meta.can_move) {
        container.innerHTML = "";
        return;
    }

    if (attachmentEditMode) {
        const attachmentParentName = attachmentEditParentMeta(meta)?.parent_node_name || "current parent";
        container.innerHTML = `<span class="topology-manager-inline-note"><i class="fa fa-tower-broadcast"></i>Editing attachment preference for ${escapeHtml(attachmentParentName)}.</span>`;
        return;
    }

    if (manualAttachmentEditMode) {
        const parentName = manualAttachmentParentMeta(meta)?.parent_node_name || "selected parent";
        container.innerHTML = `<span class="topology-manager-inline-note"><i class="fa fa-diagram-project"></i>Editing a manual attachment group for ${escapeHtml(parentName)}.</span>`;
        return;
    }

    if (attachmentRateEditAttachmentId) {
        const current = attachmentRateEditOption();
        const attachmentName = current?.option?.attachment_name || "selected attachment";
        container.innerHTML = `<span class="topology-manager-inline-note"><i class="fa fa-gauge-high"></i>Editing attachment rate override for ${escapeHtml(attachmentName)}.</span>`;
        return;
    }

    if (!moveMode) {
        container.innerHTML = "";
        return;
    }

    const proposedParentName = proposedParentMeta()?.parent_node_name || "none selected yet";
    container.innerHTML = `<span class="topology-manager-inline-note"><i class="fa fa-arrows-up-down-left-right"></i>Move mode. Green nodes are legal parents. Current target: ${escapeHtml(proposedParentName)}.</span>`;
}

function updateHeader() {
    const summary = document.getElementById("topologyManagerSummary");
    const source = document.getElementById("topologyManagerSource");
    if (!summary || !source) return;

    const movableCount = topologyNodesSorted.filter((node) => node.can_move).length;
    const overrideCount = topologyNodesSorted.filter((node) => node.has_override).length;
    const selected = selectedMeta();
    summary.textContent = selected
        ? `${selected.node_name} selected · ${movableCount} movable · ${overrideCount} saved`
        : `${movableCount} movable · ${overrideCount} saved`;
    source.textContent = topologyManagerState?.source
        ? `${topologyManagerState.source} · schema ${topologyManagerState.schema_version}`
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
    const meta = selectedMeta();
    const canonicalIds = currentPathIds(selectedNodeId);
    const liveIds = effectivePathIds(selectedNodeId);
    const savedIds = meta?.has_override
        ? proposedPathIds(selectedNodeId, meta.override_parent_node_id)
        : liveIds;
    const summaryIds = moveMode && proposedParentId
        ? proposedPathIds(selectedNodeId, proposedParentId)
        : savedIds;
    renderHierarchyPath("topologyManagerCanonicalHierarchy", canonicalIds, {
        highlightNodeId: selectedNodeId,
    });
    renderHierarchyPath("topologyManagerLiveHierarchy", liveIds, {
        highlightNodeId: selectedNodeId,
    });
    renderHierarchyPath("topologyManagerSavedHierarchy", summaryIds, {
        highlightNodeId: selectedNodeId,
    });
    const savedLabel = document.getElementById("topologyManagerSavedHierarchyLabel");
    if (savedLabel) {
        savedLabel.textContent = moveMode && proposedParentId
            ? "Proposed"
            : "Saved";
    }
    const details = document.getElementById("topologyManagerHierarchyMeta");
    if (details) {
        const rows = [];
        if (meta?.effective_parent_node_id && meta.effective_parent_node_id !== meta.current_parent_node_id) {
            rows.push("Canonical reflects the latest integration snapshot. Live reflects the runtime-effective topology used for shaping.");
        }
        if (meta?.has_override && !meta?.override_live) {
            rows.push("Saved shows operator intent. It is not live until the runtime-effective topology matches it.");
        }
        details.className = rows.length > 0 ? "topology-manager-compact-note mt-2" : "d-none";
        details.textContent = rows.join(" ");
    }

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
        } else if (attachmentRateEditAttachmentId) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Attachment Rate";
        } else if (meta?.has_override && meta?.override_live) {
            badge.className = "badge rounded-pill text-bg-success";
            badge.textContent = "Live Override";
        } else if (meta?.has_override) {
            badge.className = "badge rounded-pill text-bg-info";
            badge.textContent = "Saved Override";
        } else {
            badge.className = "badge rounded-pill text-bg-warning d-none";
        }
    }

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
                                const kindText = escapeHtml(option.attachment_kind || "attachment");
                                const roleText = attachmentRoleLabel(option);
                                const detailText = attachmentHealthDetails(option);
                                return `
                                    <div class="list-group-item topology-manager-attachment-row">
                                        <div class="d-flex flex-wrap align-items-center justify-content-between gap-2">
                                            <div class="flex-grow-1 min-w-0">
                                                <div class="d-flex flex-wrap align-items-center gap-2">
                                                    <div class="fw-semibold">${escapeHtml(option.attachment_name)}</div>
                                                    ${roleText ? `<span class="badge text-bg-light">${escapeHtml(roleText)}</span>` : ""}
                                                    ${attachmentHealthBadge(option)}
                                                    ${isEffective ? "<span class='badge text-bg-primary'>Effective</span>" : ""}
                                                    ${option.probe_enabled ? "<span class='badge text-bg-info'>Probe On</span>" : "<span class='badge text-bg-secondary'>Probe Off</span>"}
                                                </div>
                                                <div class="topology-manager-attachment-meta">${kindText}${detailText ? ` · ${detailText}` : ""}</div>
                                            </div>
                                            <div class="d-flex flex-wrap gap-2 justify-content-end flex-shrink-0">
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
    const showRateButtons = options.showRateButtons !== false && !!topologyManagerState?.writable;
    return `
        <div class="list-group topology-manager-rank-list">
            ${explicitOptions.map((option) => {
                const kindText = escapeHtml(option.attachment_kind || "attachment");
                const roleText = attachmentRoleLabel(option);
                const detailText = attachmentHealthDetails(option);
                const reasonNote = attachmentReasonNote(option);
                const selectionBadges = attachmentSelectionBadges(parentMeta, option);
                const isUsingNow = !!option.effective_selected;
                const isPreferred = selectionBadges.includes("Preferred");
                const editingRate = isEditingAttachmentRate(parentMeta, option);
                const disabledReason = option.rate_override_disabled_reason
                    ? `<div class="small text-body-secondary mt-1">${escapeHtml(option.rate_override_disabled_reason)}</div>`
                    : "";
                const rateButtonsHtml = !showRateButtons
                    ? ""
                    : option.can_override_rate
                        ? `
                            <button class="btn btn-sm ${editingRate ? "btn-primary" : "btn-outline-primary"}" type="button" data-edit-attachment-rate-parent-id="${escapeHtml(parentMeta.parent_node_id)}" data-edit-attachment-rate-id="${escapeHtml(option.attachment_id)}">
                                ${editingRate ? "Editing Rates" : option.has_rate_override ? "Edit Rates" : "Set Rates"}
                            </button>
                            ${option.has_rate_override
                                ? `<button class="btn btn-sm btn-outline-danger" type="button" data-clear-attachment-rate-parent-id="${escapeHtml(parentMeta.parent_node_id)}" data-clear-attachment-rate-id="${escapeHtml(option.attachment_id)}">
                                    Clear Rates
                                </button>`
                                : ""}`
                        : disabledReason;
                return `
                <div class="list-group-item topology-manager-attachment-row ${isUsingNow ? "topology-manager-attachment-row-active" : ""} ${!isUsingNow && isPreferred ? "topology-manager-attachment-row-preferred" : ""}">
                    <div class="d-flex flex-column gap-2">
                        <div class="d-flex flex-wrap align-items-center justify-content-between gap-2">
                            <div class="flex-grow-1 min-w-0">
                                <div class="d-flex flex-wrap align-items-center gap-2">
                                    <div class="fw-semibold">${escapeHtml(option.attachment_name)}</div>
                                    ${roleText ? `<span class="badge text-bg-light">${escapeHtml(roleText)}</span>` : ""}
                                    ${attachmentHealthBadge(option)}
                                    ${selectionBadges}
                                    ${attachmentProbeBadge(option)}
                                    ${option.has_rate_override ? "<span class='badge text-bg-warning'>Attachment Rate</span>" : ""}
                                </div>
                                <div class="topology-manager-attachment-status-line">
                                    <span>${kindText}</span>
                                    ${attachmentStatusLine(option) ? `<span>${attachmentStatusLine(option)}</span>` : ""}
                                </div>
                                ${reasonNote
                                    ? `<div class="topology-manager-attachment-note mt-1">${reasonNote}</div>`
                                    : detailText
                                        ? `<div class="topology-manager-attachment-note mt-1">${detailText}</div>`
                                        : ""}
                            </div>
                            <div class="d-flex flex-wrap gap-2 justify-content-end">
                                ${showProbeButtons && option.pair_id
                                    ? `<button class="btn btn-sm ${option.probe_enabled ? "btn-outline-secondary" : "btn-outline-info"}" type="button" data-probe-policy-pair-id="${escapeHtml(option.pair_id)}" data-probe-policy-enabled="${option.probe_enabled ? "false" : "true"}">
                                        ${option.probe_enabled ? "Turn Probe Off" : "Turn Probe On"}
                                    </button>`
                                    : ""}
                                ${rateButtonsHtml}
                            </div>
                        </div>
                        ${editingRate
                            ? `
                                <div class="border rounded p-2 bg-body-tertiary">
                                    <div class="row g-2 align-items-end">
                                        <div class="col-md-4">
                                            <label class="form-label small mb-1" for="topologyManagerAttachmentRateDownload">Download (Mbps)</label>
                                            <input class="form-control form-control-sm" id="topologyManagerAttachmentRateDownload" type="number" min="1" value="${escapeHtml(attachmentRateDraftDownloadMbps)}" placeholder="500">
                                        </div>
                                        <div class="col-md-4">
                                            <label class="form-label small mb-1" for="topologyManagerAttachmentRateUpload">Upload (Mbps)</label>
                                            <input class="form-control form-control-sm" id="topologyManagerAttachmentRateUpload" type="number" min="1" value="${escapeHtml(attachmentRateDraftUploadMbps)}" placeholder="500">
                                        </div>
                                        <div class="col-md-4">
                                            <div class="d-flex flex-wrap gap-2">
                                                <button class="btn btn-sm btn-primary" type="button" id="topologyManagerSaveAttachmentRate" ${attachmentRateSaveDisabled() ? "disabled" : ""}>
                                                    Save Rates
                                                </button>
                                                <button class="btn btn-sm btn-outline-secondary" type="button" id="topologyManagerCancelAttachmentRate">
                                                    Cancel
                                                </button>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            `
                            : ""}
                    </div>
                </div>
            `;
            }).join("")}
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
            <div class="topology-manager-section">
                <div class="d-flex flex-wrap align-items-start justify-content-between gap-2">
                    <div>
                        <div class="small text-uppercase text-body-secondary">Manual Attachment Group</div>
                        <div class="small text-body-secondary mt-1">
                            Parent <strong>${escapeHtml(parent.parent_node_name)}</strong> · ${escapeHtml(summary)} · <strong>${explicit.length}</strong> explicit attachment${explicit.length === 1 ? "" : "s"}
                        </div>
                    </div>
                    <div class="d-flex flex-wrap gap-2">
                        <button class="btn btn-outline-primary" id="topologyManagerEditManualAttachments" type="button">
                            <i class="fa fa-pen-to-square"></i> ${hasManualGroup ? "Edit Manual Group" : "Create Manual Group"}
                        </button>
                        ${hasManualGroup
                            ? `<button class="btn btn-outline-danger" id="topologyManagerClearManualAttachments" type="button">
                                <i class="fa fa-trash"></i> Clear Group
                            </button>`
                            : ""}
                    </div>
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
                                <input class="form-control form-control-sm" type="text" data-manual-row-index="${index}" data-manual-field="attachment_name" value="${escapeHtml(row.attachment_name)}" placeholder="Backhaul Attachment A">
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
    const primaryActions = document.getElementById("topologyManagerPrimaryActions");
    const clearSavedButton = document.getElementById("topologyManagerClearSaved");
    if (!container || !clearSavedButton || !primaryActions) {
        return;
    }

    const meta = selectedMeta();
    const treeNode = selectedTreeNode();
    if (!meta) {
        clearSavedButton.disabled = true;
        primaryActions.innerHTML = "";
        container.innerHTML = "<div class='topology-manager-empty'>Select a branch from the map or search box to begin.</div>";
        return;
    }

    clearSavedButton.disabled = !meta.has_override;
    const currentParent = meta.current_parent_node_name || "Root / none";
    const effectiveParent = meta.effective_parent_node_name || currentParent;
    const legalParentCount = allowedParentsForSelected().length;
    const savedOverrideText = meta.has_override
        ? `${escapeHtml(meta.override_parent_node_name || "")}${meta.override_mode === "preferred_order" && meta.override_attachment_preference_names.length > 0 ? ` via ${escapeHtml(meta.override_attachment_preference_names.join(" → "))}` : meta.override_mode === "auto" ? " (Auto)" : ""}`
        : "None";
    const attachmentParent = attachmentEditParentMeta(meta);
    const attachmentOptionCount = explicitAttachmentOptions(attachmentParent).length;
    const showAttachmentInspectEditor = !moveMode && attachmentEditMode && !!attachmentParent;
    const saveDisabled = saveProposalInFlight || !proposalIsValid() || proposalMatchesSavedOverride(meta);
    const editingDisabled = !meta.can_move;
    const currentAttachment = meta.current_attachment_name || "Auto / integration default";
    const effectiveAttachment = meta.effective_attachment_name || "Auto / integration default";
    const statusNote = editingDisabled
        ? "This node is read-only here. You can still use the preview to inspect upstream and downstream context."
        : moveMode
            ? "Choose a legal parent, optionally adjust attachment preference, then save."
            : attachmentEditMode
                ? "Adjust attachment preference for the current logical parent without changing parentage."
                : manualAttachmentEditMode
                    ? "Manual attachment-group editing is active below."
                    : attachmentRateEditAttachmentId
                        ? "Attachment rate override editing is active below."
                        : "Inspect mode only. Use Start Move to change parentage or Edit Attachment Preference to tune radios.";
    const branchSummaryPills = [
        `<span class="topology-manager-summary-pill"><span>${meta.effective_parent_node_name ? "Live parent" : "Parent"}</span><strong>${escapeHtml(effectiveParent)}</strong></span>`,
        `<span class="topology-manager-summary-pill"><span>Descendants</span><strong>${descendantCount(meta.node_id)}</strong></span>`,
        `<span class="topology-manager-summary-pill"><span>Legal parents</span><strong>${legalParentCount}</strong></span>`,
    ];
    if (meta.effective_parent_node_name && meta.effective_parent_node_name !== currentParent) {
        branchSummaryPills.push(
            `<span class="topology-manager-summary-pill"><span>Canonical parent</span><strong>${escapeHtml(currentParent)}</strong></span>`
        );
    }
    if (meta.has_override) {
        branchSummaryPills.push(
            `<span class="topology-manager-summary-pill"><span>Saved override</span><strong>${savedOverrideText}</strong></span>`
        );
    }
    if (treeNode?.runtime_virtualized) {
        branchSummaryPills.push(
            "<span class=\"topology-manager-summary-pill\"><span>Runtime</span><strong class=\"text-warning-emphasis\">Virtualized in Bakery</strong></span>"
        );
    }
    const showInlineAttachmentEditButton = !editingDisabled
        && !moveMode
        && !showAttachmentInspectEditor
        && !manualAttachmentEditMode
        && !attachmentRateEditAttachmentId
        && hasInspectableAttachmentChoices(meta);
    const attachmentPreferenceLabel = meta.preferred_attachment_name || "Auto / integration default";
    const attachmentParentSummary = attachmentParent
        ? `Upstream branch ${escapeHtml(attachmentParent.parent_node_name)}${attachmentOptionCount > 0
            ? ` with ${attachmentOptionCount} available radio path${attachmentOptionCount === 1 ? "" : "s"}`
            : " with Auto / Default only"}`
        : "No explicit radio-path options are currently available for this branch.";
    const branchTreeHref = treePageHrefForNodeId(meta.node_id);
    const branchNameHtml = branchTreeHref
        ? `<a class="fw-semibold fs-5 link-body-emphasis text-decoration-none" href="${escapeHtml(branchTreeHref)}">${escapeHtml(meta.node_name)}</a>`
        : `<div class="fw-semibold fs-5">${escapeHtml(meta.node_name)}</div>`;

    let primaryActionsHtml = "";
    if (!editingDisabled) {
        if (moveMode) {
            primaryActionsHtml = `
                <button class="btn btn-sm btn-primary" id="topologyManagerSave" type="button" ${saveDisabled ? "disabled" : ""}>
                    <i class="fa ${saveProposalInFlight ? "fa-spinner fa-spin" : "fa-save"}"></i> ${saveProposalInFlight ? "Saving..." : "Save Move"}
                </button>
                <button class="btn btn-sm btn-outline-secondary" id="topologyManagerResetInline" type="button" ${saveProposalInFlight ? "disabled" : ""}>
                    <i class="fa fa-rotate-left"></i> Reset
                </button>
                <button class="btn btn-sm btn-outline-secondary" id="topologyManagerCancelMove" type="button" ${saveProposalInFlight ? "disabled" : ""}>
                    <i class="fa fa-ban"></i> Cancel
                </button>
            `;
        } else if (showAttachmentInspectEditor) {
            primaryActionsHtml = `
                <button class="btn btn-sm btn-primary" id="topologyManagerSave" type="button" ${saveDisabled ? "disabled" : ""}>
                    <i class="fa ${saveProposalInFlight ? "fa-spinner fa-spin" : "fa-save"}"></i> ${saveProposalInFlight ? "Saving..." : "Save Preference"}
                </button>
                <button class="btn btn-sm btn-outline-secondary" id="topologyManagerResetInline" type="button" ${saveProposalInFlight ? "disabled" : ""}>
                    <i class="fa fa-rotate-left"></i> Reset
                </button>
                <button class="btn btn-sm btn-outline-secondary" id="topologyManagerCancelAttachmentEdit" type="button" ${saveProposalInFlight ? "disabled" : ""}>
                    <i class="fa fa-ban"></i> Cancel
                </button>
            `;
        } else if (!manualAttachmentEditMode && !attachmentRateEditAttachmentId) {
            primaryActionsHtml = `
                ${hasInspectableAttachmentChoices(meta) && !attachmentEditMode
                    ? `
                        <button class="btn btn-sm btn-primary" id="topologyManagerEditAttachmentPreference" type="button">
                            <i class="fa fa-tower-broadcast"></i> Edit Link Path
                        </button>
                    `
                    : ""}
                <button class="btn btn-sm btn-outline-secondary" id="topologyManagerStartMove" type="button" ${legalParentCount === 0 ? "disabled" : ""}>
                    <i class="fa fa-arrows-up-down-left-right"></i> Start Move
                </button>
            `;
        }
    }
    primaryActions.innerHTML = primaryActionsHtml;

    container.innerHTML = `
        ${(meta.warnings || []).map((warning) => `<div class="alert alert-warning py-2 mb-0">${escapeHtml(warning)}</div>`).join("")}

        <div class="topology-manager-section">
            <div class="topology-manager-section-header">
                <div class="topology-manager-section-header-main">
                    <div class="small text-uppercase text-body-secondary">Branch</div>
                    ${branchNameHtml}
                    <div class="small text-body-secondary">${escapeHtml(nodeKindLabel(meta.node_id))}</div>
                </div>
                <span class="badge ${meta.can_move ? "text-bg-info" : "text-bg-secondary"}">${meta.can_move ? "Movable" : "Read Only"}</span>
            </div>
            <div class="topology-manager-summary-meta">
                ${branchSummaryPills.join("")}
            </div>
            <div class="topology-manager-compact-note"><i class="fa fa-circle-info me-1"></i>${escapeHtml(statusNote)}</div>
        </div>

        ${editingDisabled
            ? ""
            : `
                ${moveMode
                    ? `
                        <div class="topology-manager-section">
                            <div class="small text-uppercase text-body-secondary mb-2">Choose New Parent</div>
                            ${renderTargetCards(meta)}
                        </div>

                        <div class="topology-manager-section">
                            <div class="small text-uppercase text-body-secondary mb-2">Attachment / Radio Preferences</div>
                            ${renderAttachmentEditor(meta)}
                        </div>
                    `
                    : `
                        <div class="topology-manager-section">
                            <div class="topology-manager-section-header">
                                <div class="topology-manager-section-header-main">
                                    <div class="small text-uppercase text-body-secondary">Radio Paths</div>
                                    <div class="topology-manager-section-subtitle">${attachmentParentSummary}</div>
                                </div>
                            </div>
                            <div class="topology-manager-kv-grid">
                                <div class="topology-manager-kv-row"><span>Using now</span><strong>${escapeHtml(currentAttachment)}</strong></div>
                                <div class="topology-manager-kv-row"><span>Preferred</span><strong>${escapeHtml(attachmentPreferenceLabel)}</strong></div>
                                ${currentAttachment === effectiveAttachment
                                    ? ""
                                    : `<div class="topology-manager-kv-row"><span>Will apply</span><strong>${escapeHtml(effectiveAttachment)}</strong></div>`}
                            </div>
                            ${attachmentParent
                                ? `
                                    <div>
                                        <div class="d-flex flex-wrap align-items-center justify-content-between gap-2 mb-2">
                                            <div class="small text-uppercase text-body-secondary mb-0">Attachment Health</div>
                                            <a class="btn btn-sm btn-outline-secondary" href="topology_probes.html">
                                                <i class="fa fa-wave-square"></i> Probe Debug
                                            </a>
                                        </div>
                                        ${renderAttachmentHealthList(attachmentParent)}
                                    </div>
                                `
                                : "<div class='topology-manager-compact-note'>No attachment-health rows are available for this branch yet.</div>"}
                            ${showAttachmentInspectEditor
                                ? `
                                    <div class="pt-1 border-top border-secondary-subtle">
                                        <div class="small text-uppercase text-body-secondary mb-2">Edit Link Path</div>
                                        ${renderAttachmentEditor(meta)}
                                    </div>
                                `
                                : ""}
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

    const currentPath = truncatedPreviewPath(currentPathIds(meta.node_id), `current:${meta.node_id}`);
    const allowedParents = moveMode ? allowedParentsForSelected() : [];
    const alternatePaths = allowedParents.map((parent) => ({
        parent,
        path: truncatedPreviewPath(currentPathIds(parent.parent_node_id), `alternate:${parent.parent_node_id}`),
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

    currentPath.forEach((node, depth) => {
        displayedNodes.set(node.nodeId, {
            ...node,
            nodeId: node.nodeId,
            x: startX + (depth * xStep),
            y: MAP_CENTER_Y,
            depth,
            lane: 0,
            role: "current",
        });
        if (depth > 0) {
            displayedEdges.push({
                from: currentPath[depth - 1].nodeId,
                to: node.nodeId,
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
        entry.path.forEach((node, depth) => {
            if (displayedNodes.has(node.nodeId)) {
                return;
            }
            displayedNodes.set(node.nodeId, {
                ...node,
                nodeId: node.nodeId,
                x: startX + (depth * xStep),
                y: MAP_CENTER_Y + (lane * LANE_STEP),
                depth,
                lane,
                role: "alternate",
            });
        });
        for (let depth = 1; depth < entry.path.length; depth += 1) {
            displayedEdges.push({
                from: entry.path[depth - 1].nodeId,
                to: entry.path[depth].nodeId,
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
        const isSelectable = !node.isSynthetic && canSelectNodeId(node.nodeId);
        const isEditable = topologyNodeById.has(node.nodeId);
        const label = truncateLabel(node.label || displayNameForNodeId(node.nodeId), 22);
        const sublabel = node.sublabel || (isEditable
            ? nodeKindLabel(node.nodeId)
            : node.nodeId === ROOT_NODE_ID
                ? "Root Context"
                : `${nodeKindLabel(node.nodeId)} Context`);
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
        if (node.isSynthetic) {
            fill = "rgba(15, 23, 42, 0.48)";
            stroke = "rgba(148, 163, 184, 0.36)";
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

        const interactiveAttrs = isSelectable
            ? `data-map-node-id="${escapeHtml(node.nodeId)}" onclick="window.topologyManagerSelectNode(this.getAttribute('data-map-node-id')); event.stopPropagation();"`
            : "";

        return `
            <g class="topology-map-node" ${interactiveAttrs} transform="translate(${node.x}, ${node.y})" opacity="${opacity}" style="cursor:${cursor};">
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

async function saveAttachmentRateOverride() {
    const current = attachmentRateEditOption();
    if (!current) {
        return;
    }
    syncAttachmentRateDraftFromInputs();
    const download = Number.parseInt(String(attachmentRateDraftDownloadMbps || ""), 10);
    const upload = Number.parseInt(String(attachmentRateDraftUploadMbps || ""), 10);
    if (!Number.isFinite(download) || download <= 0 || !Number.isFinite(upload) || upload <= 0) {
        showWarnings(["Attachment rate overrides require positive download and upload Mbps values."], null);
        return;
    }

    try {
        const response = await sendWsRequest("SetTopologyManagerAttachmentRateOverrideResult", {
            SetTopologyManagerAttachmentRateOverride: {
                update: {
                    child_node_id: selectedMeta()?.node_id,
                    parent_node_id: current.parent.parent_node_id,
                    attachment_id: current.option.attachment_id,
                    download_bandwidth_mbps: download,
                    upload_bandwidth_mbps: upload,
                },
            },
        });
        indexTopologyState(response.data);
        clearAttachmentRateEditState();
        lastFlash = "Attachment rate override saved.";
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to save attachment rate override."], null);
    }
}

async function clearAttachmentRateOverride(parentNodeId, attachmentId) {
    const meta = selectedMeta();
    if (!meta || !parentNodeId || !attachmentId) {
        return;
    }
    try {
        const response = await sendWsRequest("ClearTopologyManagerAttachmentRateOverrideResult", {
            ClearTopologyManagerAttachmentRateOverride: {
                clear: {
                    child_node_id: meta.node_id,
                    parent_node_id: parentNodeId,
                    attachment_id: attachmentId,
                },
            },
        });
        indexTopologyState(response.data);
        if (attachmentRateEditParentId === parentNodeId && attachmentRateEditAttachmentId === attachmentId) {
            clearAttachmentRateEditState();
        }
        lastFlash = "Attachment rate override cleared.";
        renderEverything(response.data.global_warnings || []);
    } catch (error) {
        showWarnings([error?.message || "Unable to clear attachment rate override."], null);
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
    clearAttachmentRateEditState();
    manualAttachmentDraftRows = cloneManualAttachmentRows(parent);
    if (manualAttachmentDraftRows.length === 0) {
        manualAttachmentDraftRows = [blankManualAttachmentRow(), blankManualAttachmentRow()];
    }
    renderEverything();
}

function cancelManualAttachmentEditMode() {
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    clearAttachmentRateEditState();
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
        clearAttachmentRateEditState();
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
        clearAttachmentRateEditState();
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
    document.querySelectorAll("[data-edit-attachment-rate-parent-id]").forEach((button) => {
        button.addEventListener("click", () => {
            const parentId = button.getAttribute("data-edit-attachment-rate-parent-id");
            const attachmentId = button.getAttribute("data-edit-attachment-rate-id");
            if (!parentId || !attachmentId) {
                return;
            }
            startAttachmentRateEdit(parentId, attachmentId);
        });
    });
    document.querySelectorAll("[data-clear-attachment-rate-parent-id]").forEach((button) => {
        button.addEventListener("click", async () => {
            const parentId = button.getAttribute("data-clear-attachment-rate-parent-id");
            const attachmentId = button.getAttribute("data-clear-attachment-rate-id");
            if (!parentId || !attachmentId) {
                return;
            }
            await clearAttachmentRateOverride(parentId, attachmentId);
        });
    });
    document.getElementById("topologyManagerSaveAttachmentRate")?.addEventListener("click", () => {
        saveAttachmentRateOverride();
    });
    document.getElementById("topologyManagerCancelAttachmentRate")?.addEventListener("click", () => {
        cancelAttachmentRateEditMode();
    });
    document.getElementById("topologyManagerAttachmentRateDownload")?.addEventListener("input", () => {
        syncAttachmentRateDraftFromInputs();
        refreshAttachmentRateSaveButton();
    });
    document.getElementById("topologyManagerAttachmentRateUpload")?.addEventListener("input", () => {
        syncAttachmentRateDraftFromInputs();
        refreshAttachmentRateSaveButton();
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

function setSelectedNode(nodeId, options = {}) {
    if (!canSelectNodeId(nodeId)) {
        return;
    }
    clearSearchSuggestions();
    selectedNodeId = nodeId;
    moveMode = false;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    clearAttachmentRateEditState();
    resetMapView();
    syncSelectedNodeUrl(selectedNodeId, options.historyMode || "push");
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
    clearAttachmentRateEditState();
    initializeProposalFromSaved(meta);
    renderEverything();
}

function cancelMoveMode() {
    moveMode = false;
    attachmentEditMode = false;
    manualAttachmentEditMode = false;
    manualAttachmentDraftRows = [];
    clearAttachmentRateEditState();
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
    clearAttachmentRateEditState();
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
    clearAttachmentRateEditState();
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
    if (saveProposalInFlight || !proposalIsValid() || !selectedNodeId || !proposedParentId) {
        return;
    }

    saveProposalInFlight = true;
    renderEverything();
    let warningsToRender = null;
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
        clearAttachmentRateEditState();
        lastFlash = "Topology override saved and published to the runtime-effective topology.";
        if (selectedNodeId && topologyNodeById.has(selectedNodeId)) {
            initializeProposalFromSaved(selectedMeta());
        }
        warningsToRender = response.data.global_warnings || [];
    } catch (error) {
        warningsToRender = [error?.message || "Unable to save topology override."];
    } finally {
        saveProposalInFlight = false;
        renderEverything(warningsToRender);
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
        clearAttachmentRateEditState();
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
        applyTopologyManagerState(stateResponse.data, {
            preferredNodeId: preferredInitialNodeId(),
            flash: null,
        });
        scheduleTopologyStateRefresh();
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
document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
        return;
    }
    scheduleTopologyStateRefresh(TOPOLOGY_STATE_REFRESH_VISIBLE_DELAY_MS);
});
window.addEventListener("popstate", () => {
    if (!topologyManagerState) {
        return;
    }
    const requestedNodeId = preferredInitialNodeId();
    const nextNodeId = canSelectNodeId(requestedNodeId) ? requestedNodeId : pickDefaultNodeId();
    if (!canSelectNodeId(nextNodeId) || nextNodeId === selectedNodeId) {
        return;
    }
    setSelectedNode(nextNodeId, {historyMode: "replace"});
});
window.topologyManagerSelectNode = selectNodeFromMap;
listenOnce("join", () => {
    loadPage();
});
loadPage();
