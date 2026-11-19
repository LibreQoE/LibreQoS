// Pure transforms for the CPU & Workers page. No DOM, no fetch: node --test
// covers the honest-empty/absent semantics here, and the entrypoint only
// renders the normalized shape. The server's reason codes stay codes; the words
// this module shows for them come from the active catalog.

import {t, tn} from "./i18n/index.js";

const WORKER_COLORS = [
    "#4e79a7", "#f28e2b", "#59a14f", "#b07aa1", "#76b7b2",
    "#edc948", "#ff9da7", "#9c755f", "#bab0ac", "#e15759",
];

export function isSectionAvailable(section) {
    return !!section && section.available === true;
}

function integerOrNull(value) {
    return Number.isInteger(value) ? value : null;
}

function stringOrNull(value) {
    return typeof value === "string" && value.length > 0 ? value : null;
}

function boolOrNull(value) {
    return typeof value === "boolean" ? value : null;
}

function integerListOrNull(value) {
    if (!Array.isArray(value)) return null;
    return value.filter((entry) => Number.isInteger(entry));
}

function normalizeCpuUsage(section) {
    if (!isSectionAvailable(section) || !Array.isArray(section.per_cpu)) {
        return {
            available: false,
            reason: stringOrNull(section?.reason) || "cpu_usage_unavailable",
            numCpus: null,
            rows: [],
        };
    }
    const rows = section.per_cpu.map((row, index) => {
        const cpuId = integerOrNull(row?.cpu_id);
        return {
            cpuId,
            key: cpuId === null ? `index-${index}` : `cpu-${cpuId}`,
            label: cpuId === null ? t("cpu_workers.cpu_unknown") : `CPU ${cpuId}`,
            percent: Number.isFinite(row?.percent) ? row.percent : null,
        };
    });
    return {
        available: true,
        reason: null,
        numCpus: integerOrNull(section.num_cpus) ?? rows.length,
        rows,
    };
}

function normalizeTopology(section) {
    if (!isSectionAvailable(section)) {
        return {
            available: false,
            reason: stringOrNull(section?.reason) || "cpu_topology_unavailable",
            excludeEfficiencyCores: null,
            source: null,
            fromCache: null,
            hasHybridSplit: null,
            possible: null,
            performance: null,
            efficiency: null,
            shaping: null,
            detail: null,
        };
    }
    return {
        available: true,
        reason: null,
        excludeEfficiencyCores: boolOrNull(section.exclude_efficiency_cores),
        source: stringOrNull(section.source),
        fromCache: boolOrNull(section.from_cache),
        hasHybridSplit: boolOrNull(section.has_hybrid_split),
        possible: integerListOrNull(section.possible),
        performance: integerListOrNull(section.performance),
        efficiency: integerListOrNull(section.efficiency),
        shaping: integerListOrNull(section.shaping),
        detail: stringOrNull(section.detail),
    };
}

function normalizeWorkers(section) {
    if (!isSectionAvailable(section) || !Array.isArray(section.rows)) {
        return {
            available: false,
            reason: stringOrNull(section?.reason) || "bridge_report_unavailable",
            schema: null,
            rxQueueCount: null,
            txQueueCount: null,
            counterSupport: null,
            rows: [],
        };
    }
    const rows = section.rows.map((row, index) => {
        const workerId = integerOrNull(row?.worker_id);
        return {
            key: workerId === null ? `worker-${index}` : `worker-${workerId}`,
            workerId,
            lcoreId: integerOrNull(row?.lcore_id),
            numaNode: integerOrNull(row?.numa_node),
            rxQueueIds: integerListOrNull(row?.rx_queue_ids),
            txQueueId: integerOrNull(row?.tx_queue_id),
            sharedTxQueue: boolOrNull(row?.shared_tx_queue),
            circuitCount: integerOrNull(row?.circuit_count),
            solelyOwnedNodeCount: integerOrNull(row?.solely_owned_node_count),
            counters: row?.counters ?? null,
            throughput: Number.isFinite(row?.throughput) ? row.throughput : null,
            throughputSupport: stringOrNull(row?.throughput_support)
                || (Number.isFinite(row?.throughput) ? "available" : "unsupported"),
            drops: Number.isFinite(row?.drops) ? row.drops : null,
            dropsSupport: stringOrNull(row?.drops_support)
                || (Number.isFinite(row?.drops) ? "available" : "unsupported"),
        };
    });
    return {
        available: true,
        reason: null,
        schema: integerOrNull(section.schema),
        rxQueueCount: integerOrNull(section.rx_queue_count),
        txQueueCount: integerOrNull(section.tx_queue_count),
        counterSupport: stringOrNull(section.counter_support),
        rows,
    };
}

function unavailableOwnership(reason) {
    return {
        available: false,
        reason,
        generation: null,
        circuits: null,
        nodes: null,
        crossings: null,
        unownedCircuits: null,
    };
}

function normalizeOwnership(section) {
    if (!isSectionAvailable(section)) {
        return unavailableOwnership(
            stringOrNull(section?.reason) || "owner_map_unavailable",
        );
    }
    if (
        !Array.isArray(section.circuits)
        || !Array.isArray(section.nodes)
        || !Array.isArray(section.crossings)
    ) {
        return unavailableOwnership("ownership_payload_incomplete");
    }
    const circuits = Array.isArray(section.circuits)
        ? section.circuits.map((circuit) => ({
            circuitId: String(circuit?.circuit_id ?? ""),
            parentNodeId: stringOrNull(circuit?.parent_node_id),
            name: stringOrNull(circuit?.name),
            workerId: integerOrNull(circuit?.worker_id),
        })).filter((circuit) => circuit.circuitId !== "")
        : [];
    const nodes = Array.isArray(section.nodes)
        ? section.nodes.map((node) => ({
            nodeId: String(node?.node_id ?? ""),
            parentNodeId: stringOrNull(node?.parent_node_id),
            name: stringOrNull(node?.name),
            workerIds: integerListOrNull(node?.worker_ids) ?? [],
            crossing: node?.crossing === true,
        })).filter((node) => node.nodeId !== "")
        : [];
    const crossings = Array.isArray(section.crossings)
        ? section.crossings.filter((entry) => typeof entry === "string")
        : [];
    return {
        available: true,
        reason: null,
        generation: stringOrNull(section.generation),
        circuits,
        nodes,
        crossings,
        unownedCircuits: integerOrNull(section.unowned_circuits),
    };
}

function unavailableMoves(reason) {
    return {
        available: false,
        reason,
        source: null,
        sinceUnix: null,
        items: null,
    };
}

function normalizeMoves(section) {
    if (!isSectionAvailable(section)) {
        return unavailableMoves(
            stringOrNull(section?.reason) || "no_history_source_v1",
        );
    }
    if (!Array.isArray(section.items)) {
        return unavailableMoves("moves_payload_incomplete");
    }
    const items = Array.isArray(section.items)
        ? section.items.map((item) => ({
            circuitId: String(item?.circuit_id ?? ""),
            name: stringOrNull(item?.name),
            fromWorker: integerOrNull(item?.from_worker),
            toWorker: integerOrNull(item?.to_worker),
            generation: stringOrNull(item?.generation),
            observedAtUnix: integerOrNull(item?.observed_at_unix),
            source: stringOrNull(item?.source),
            cause: stringOrNull(item?.cause),
        })).filter((item) => item.circuitId !== "")
        : [];
    return {
        available: true,
        reason: null,
        source: stringOrNull(section.source),
        sinceUnix: integerOrNull(section.since_unix),
        items,
    };
}

/// 74t171: the additive per-worker utilization section. An absent/old
/// payload is an explicit unavailable state (never fabricated zeros).
export function normalizeUtilization(section) {
    if (!section || section.available !== true || !Array.isArray(section.rows)) {
        return {
            available: false,
            reason: typeof section?.reason === "string" ? section.reason : "unavailable",
            rows: [],
        };
    }
    return {available: true, reason: null, rows: section.rows};
}

export function normalizeCpuWorkers(payload) {
    const source = payload && typeof payload === "object" ? payload : {};
    let state = ["ok", "partial", "unavailable"].includes(source.state)
        ? source.state
        : "unavailable";
    let unavailableReasons = Array.isArray(source.unavailable_reasons)
        ? source.unavailable_reasons.filter((reason) => typeof reason === "string")
        : [];
    const cpuUsage = normalizeCpuUsage(source.cpu_usage);
    const topology = normalizeTopology(source.cpu_topology);
    const workers = normalizeWorkers(source.workers);
    const ownership = normalizeOwnership(source.ownership);
    const moves = normalizeMoves(source.moves);
    const utilization = normalizeUtilization(source.utilization);
    // Move history is intentionally unsupported in v1. Don't let that one
    // expected absence turn an otherwise healthy page into "Partial", and
    // don't repeat the same message in the top reason strip.
    const requiredAvailable = cpuUsage.available
        && topology.available
        && source.bridge?.available === true
        && workers.available
        && ownership.available;
    if (state === "partial"
        && requiredAvailable
        && unavailableReasons.length > 0
        && unavailableReasons.every((reason) => reason === "no_history_source_v1")
    ) {
        state = "ok";
        unavailableReasons = [];
    }
    return {
        state,
        unavailableReasons,
        generatedAtUnix: Number.isFinite(source.generated_at_unix)
            ? source.generated_at_unix
            : null,
        cpuUsage,
        topology,
        workers,
        ownership,
        moves,
        utilization,
    };
}

/// Per-section content signatures for the refresh guard. Timestamps
/// (`generated_at_unix`, `moves.since_unix`) are excluded, and a section that
/// did not change does not invalidate another section. This is what prevents a
/// CPU-usage tick from re-rendering the 1,165-node ownership tree.
export function sectionSignatures(payload) {
    const source = payload && typeof payload === "object" ? payload : {};
    const moves = source.moves && typeof source.moves === "object"
        ? {
            available: source.moves.available,
            reason: source.moves.reason,
            source: source.moves.source,
            items: source.moves.items,
        }
        : null;
    return {
        state: JSON.stringify({
            state: source.state ?? null,
            unavailable_reasons: source.unavailable_reasons ?? null,
        }),
        cpuUsage: JSON.stringify(source.cpu_usage ?? null),
        topology: JSON.stringify(source.cpu_topology ?? null),
        workers: JSON.stringify({
            workers: source.workers ?? null,
            ownership: source.ownership ?? null,
        }),
        tree: JSON.stringify({
            ownership: source.ownership ?? null,
            topology: source.cpu_topology ?? null,
        }),
        moves: JSON.stringify(moves),
        utilization: JSON.stringify(source.utilization ?? null),
    };
}

/// The heat scale's colour, from a chroma triple and an alpha. Both are data the
/// page draws with, never display text.
function heatColor(channels, alpha) {
    // translatable: false — "rgba( , )" is the CSS colour function's own syntax
    return `rgba(${channels}, ${alpha.toFixed(3)})`;
}

/// Neutral/heat scale for the CPU cells: green (cool) -> amber -> red (hot).
/// `null` for an unproven/absent percentage; never a fabricated zero.
export function cpuHeatColor(percent) {
    if (!Number.isFinite(percent)) return null;
    const value = Math.max(0, Math.min(100, percent));
    if (value >= 80) {
        return heatColor("220, 53, 69", 0.22 + (value / 100) * 0.5);
    }
    if (value >= 55) {
        return heatColor("255, 193, 7", 0.20 + (value / 100) * 0.5);
    }
    return heatColor("25, 135, 84", 0.16 + (value / 100) * 0.45);
}

const ASSEMBLED_SECTIONS = [
    "cpu_usage",
    "cpu_topology",
    "bridge",
    "workers",
    "ownership",
    "moves",
];

function isPlainObject(value) {
    return !!value && typeof value === "object" && !Array.isArray(value);
}

/// The page consumes the ASSEMBLED .118.2 shape, not the raw `GetCpuWorkers`
/// bus document. A raw/unknown shape is a protocol error: it must never render
/// as "Unavailable" with "All reported sources are present".
export function isAssembledCpuWorkersPayload(payload) {
    if (!isPlainObject(payload)) return false;
    if (!["ok", "partial", "unavailable"].includes(payload.state)) return false;
    if (!Array.isArray(payload.unavailable_reasons)) return false;
    return ASSEMBLED_SECTIONS.every((key) => isPlainObject(payload[key]));
}

export function formatCount(value) {
    return Number.isInteger(value) ? String(value) : "—";
}

/// True when the wire gave no usable display name. The tree sort reads this flag
/// instead of comparing against a rendered word, which the catalog can translate.
function hasNoName(node) {
    return (typeof node?.name === "string" ? node.name.trim() : "") === "";
}

/// Site/AP display name. A missing or blank name is the literal fallback —
/// the stable node id is NEVER rendered as a name.
export function siteName(node) {
    const name = typeof node?.name === "string" ? node.name.trim() : "";
    return name || t("cpu_workers.unknown_site");
}

/// Circuit display name. A missing/blank name is the literal fallback — the
/// stable circuit id is NEVER rendered as a name.
export function circuitName(circuit) {
    const name = typeof circuit?.name === "string" ? circuit.name.trim() : "";
    return name || t("cpu_workers.unknown_circuit");
}

/// Human text for the API's stable reason codes. Never show the raw code; a code
/// younger than this table degrades to its humanized form, never to a catalog key.
export function reasonText(reason) {
    if (!reason) return t("cpu_workers.state.unavailable");
    switch (reason) {
    case "owner_map_unavailable":
        return t("cpu_workers.reason.owner_map_unavailable");
    case "no_history_source_v1":
        return t("cpu_workers.reason.no_history_source_v1");
    case "bridge_report_unavailable":
        return t("cpu_workers.reason.bridge_report_unavailable");
    case "cpu_usage_unavailable":
        return t("cpu_workers.reason.cpu_usage_unavailable");
    case "cpu_topology_unavailable":
        return t("cpu_workers.reason.cpu_topology_unavailable");
    case "moves_payload_incomplete":
        return t("cpu_workers.reason.moves_payload_incomplete");
    case "ownership_payload_incomplete":
        return t("cpu_workers.reason.ownership_payload_incomplete");
    default:
        return String(reason).replaceAll("_", " ");
    }
}

/// Compact headline for the shaping-topology summary card.
export function topologyHeadline(topology) {
    if (!topology?.available) return t("cpu_workers.state.unavailable");
    if (topology.hasHybridSplit === true) return t("cpu_workers.topology.hybrid");
    const count = topology.shaping?.length ?? topology.possible?.length;
    if ((topology.source || "").startsWith("fallback")) {
        return count !== null && count !== undefined
            ? tn("cpu_workers.topology.all_cpus_fallback", count)
            : t("cpu_workers.topology.all_cpus_fallback_unknown");
    }
    return count !== null && count !== undefined
        ? tn("cpu_workers.topology.shaping_cpus", count)
        : t("cpu_workers.topology.available");
}

/// Synthetic DOM ids for the rendered node list: `cpu-node-<index>` in the
/// caller's node order. Stable for the same order, unique within a render, and
/// independent of the stable site id (which never reaches the DOM/href).
export function nodeDomIds(nodes) {
    const ids = new Map();
    (Array.isArray(nodes) ? nodes : []).forEach((node, index) => {
        if (node?.nodeId) ids.set(node.nodeId, `cpu-node-${index}`);
    });
    return ids;
}

/// Build the render model for ownership: named nodes/tree, named crossings and
/// named circuit rows. Stable ids remain internal keys only; every rendered
/// name falls back to the explicit Unknown literal.
export function ownershipView(ownership) {
    if (!ownership?.available) {
        return {
            available: false,
            reason: ownership?.reason || "owner_map_unavailable",
            nodes: [],
            tree: [],
            crossings: [],
            circuits: [],
        };
    }
    const nodes = ownership.nodes.map((node) => ({...node}));
    const domIds = nodeDomIds(nodes);
    const nodeById = new Map(nodes.map((node) => [node.nodeId, node]));
    const namedNodes = nodes.map((node) => ({
        ...node,
        name: siteName(node),
        unknownName: hasNoName(node),
        domId: domIds.get(node.nodeId),
    }));
    const tree = buildOwnershipTree(namedNodes);
    const crossings = ownership.crossings.map((id) => {
        const node = nodeById.get(id);
        return {
            domId: domIds.get(id) ?? null,
            name: siteName(node),
            workerIds: Array.isArray(node?.workerIds) ? node.workerIds : [],
        };
    });
    const circuits = ownership.circuits.map((circuit) => ({
        name: circuitName(circuit),
        siteName: siteName(nodeById.get(circuit.parentNodeId)),
        workerId: circuit.workerId,
    }));
    return {
        available: true,
        reason: null,
        nodes: namedNodes,
        tree,
        crossings,
        circuits,
    };
}

export function formatPercent(value) {
    return Number.isFinite(value) ? `${Math.round(value)}%` : "—";
}

export function formatQueueIds(value) {
    if (value === null || value === undefined) return "—";
    if (!Array.isArray(value)) return "—";
    return value.length === 0 ? t("cpu_workers.value.none") : value.join(", ");
}

export function formatBool(value) {
    if (value === true) return t("cpu_workers.value.yes");
    if (value === false) return t("cpu_workers.value.no");
    return "—";
}

/// Never turns an absent per-worker metric into 0. `support === "unsupported"`
/// (or an absent value without a numeric reading) renders the explicit marker.
export function formatMetric(value, support) {
    if (support === "unsupported") return t("cpu_workers.value.unsupported");
    if (value === null || value === undefined) {
        return support === "available"
            ? t("cpu_workers.value.unavailable")
            : t("cpu_workers.value.unsupported");
    }
    return Number.isFinite(value) ? String(value) : t("cpu_workers.value.unavailable");
}

export function workerColor(workerId) {
    if (!Number.isInteger(workerId) || workerId < 0) {
        // translatable: false — "var(--bs-secondary)" is a Bootstrap CSS custom property
        return "var(--bs-secondary)";
    }
    return WORKER_COLORS[workerId % WORKER_COLORS.length];
}

/// A site/node whose descendant circuits span more than one worker is SHARDED.
/// The API already propagates every leaf owner to all ancestors, so
/// `worker_ids.length > 1` (and `crossing:true`) is the authoritative signal.
export function shardedSiteLabel(node) {
    if (!node || !Array.isArray(node.workerIds) || node.workerIds.length <= 1) {
        return null;
    }
    return tn("cpu_workers.topology.sharded_across", node.workerIds.length);
}

/// Per-site worker count for sites that span more than one worker.
/// Names are the operator deduped display names; a site absent from the map is
/// not sharded (or unresolved, in which case the row simply shows no badge).
export function shardedSiteWorkerCounts(nodes) {
    const counts = new Map();
    for (const node of Array.isArray(nodes) ? nodes : []) {
        const name = typeof node?.name === "string" ? node.name.trim() : "";
        const workers = Array.isArray(node?.workerIds) ? node.workerIds.length : 0;
        if (name && workers > 1) counts.set(name, workers);
    }
    return counts;
}

/// Count of sharded sites/nodes. `null` when ownership is unavailable — the UI
/// must never imply "not sharded" without the owner map.
export function countShardedSites(nodes) {
    if (!Array.isArray(nodes)) return null;
    return nodes.filter((node) => Array.isArray(node.workerIds) && node.workerIds.length > 1).length;
}

/// Display value for the sharded-site count. `null`/unavailable ownership is
/// reported as its reason, never as 0 ("not sharded" requires the owner map).
export function shardedSummary(ownership) {
    if (!ownership || ownership.available !== true) {
        return reasonText(ownership?.reason || "owner_map_unavailable");
    }
    return String(countShardedSites(ownership.nodes));
}

export function stateLabel(state) {
    switch (state) {
    case "ok":
        return t("cpu_workers.state.healthy");
    case "partial":
        return t("cpu_workers.state.partial");
    case "unavailable":
        return t("cpu_workers.state.unavailable");
    case "protocol_error":
        return t("cpu_workers.protocol_error");
    default:
        return t("cpu_workers.state.unknown");
    }
}

export function stateBadgeClass(state) {
    switch (state) {
    case "ok":
        return "bg-success";
    case "partial":
        return "bg-warning text-dark";
    case "unavailable":
        return "bg-secondary";
    case "protocol_error":
        return "bg-danger";
    default:
        return "bg-secondary";
    }
}

export function topologySummary(topology) {
    if (!topology?.available) {
        return reasonText(topology?.reason || "cpu_topology_unavailable");
    }
    // Each entry is a complete short label; the separator is the page's own.
    const parts = [];
    if (topology.source) parts.push(t("cpu_workers.topology.source", {source: topology.source}));
    if (topology.hasHybridSplit === true) parts.push(t("cpu_workers.topology.hybrid_split"));
    if (topology.hasHybridSplit === false) parts.push(t("cpu_workers.topology.no_hybrid_split"));
    if (topology.excludeEfficiencyCores === true) parts.push(t("cpu_workers.topology.ecore_exclusion"));
    if (topology.possible !== null) {
        parts.push(t("cpu_workers.topology.possible", {count: topology.possible.length}));
    }
    if (topology.shaping !== null) {
        parts.push(t("cpu_workers.topology.shaping", {count: topology.shaping.length}));
    }
    if (topology.fromCache === true) parts.push(t("cpu_workers.topology.from_cache"));
    return parts.length ? parts.join(" · ") : t("cpu_workers.topology.available");
}

export function movesSummary(moves) {
    if (!moves?.available) return reasonText(moves?.reason || "no_history_source_v1");
    if (!moves.items.length) return t("cpu_workers.moves.none");
    return t("cpu_workers.moves.observed", {count: moves.items.length});
}

function hasParentCycle(node, byId) {
    const seen = new Set();
    let current = node;
    while (current && current.parentNodeId) {
        if (seen.has(current.nodeId)) return true;
        seen.add(current.nodeId);
        current = byId.get(current.parentNodeId);
    }
    return false;
}

/// Build a root-first tree from the flat node rows. A missing/self parent is a
/// root; a parent cycle is broken to a root (never an infinite render).
export function buildOwnershipTree(nodes) {
    const list = Array.isArray(nodes) ? nodes.filter((node) => node?.nodeId) : [];
    const byId = new Map(list.map((node) => [node.nodeId, {...node, children: []}]));
    for (const node of byId.values()) {
        if (node.parentNodeId && hasParentCycle(node, byId)) node.parentNodeId = null;
        if (node.parentNodeId === node.nodeId) node.parentNodeId = null;
    }
    const roots = [];
    for (const node of byId.values()) {
        const parent = node.parentNodeId ? byId.get(node.parentNodeId) : null;
        if (parent && parent !== node) parent.children.push(node);
        else roots.push(node);
    }
    const seen = new Set();
    const sort = (a, b) => {
        const aUnknown = a.unknownName === true ? 1 : 0;
        const bUnknown = b.unknownName === true ? 1 : 0;
        return aUnknown - bUnknown
            || String(a.name || "").localeCompare(String(b.name || ""))
            || a.nodeId.localeCompare(b.nodeId);
    };
    const finalize = (node) => {
        if (seen.has(node.nodeId)) return;
        seen.add(node.nodeId);
        node.children.sort(sort);
        for (const child of node.children) finalize(child);
    };
    roots.sort(sort).forEach(finalize);
    for (const node of byId.values()) {
        if (!seen.has(node.nodeId)) {
            roots.push(node);
            finalize(node);
        }
    }
    return roots;
}

export function flattenOwnershipTree(roots, depth = 0) {
    const rows = [];
    for (const node of roots || []) {
        rows.push({...node, depth});
        rows.push(...flattenOwnershipTree(node.children, depth + 1));
    }
    return rows;
}
