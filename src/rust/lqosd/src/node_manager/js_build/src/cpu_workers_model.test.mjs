import assert from "node:assert/strict";
import test from "node:test";

import {
    buildOwnershipTree,
    circuitName,
    countShardedSites,
    shardedSiteWorkerCounts,
    cpuHeatColor,
    nodeDomIds,
    flattenOwnershipTree,
    formatCount,
    formatMetric,
    formatQueueIds,
    movesSummary,
    normalizeCpuWorkers,
    ownershipView,
    reasonText,
    shardedSiteLabel,
    isAssembledCpuWorkersPayload,
    sectionSignatures,
    shardedSummary,
    siteName,
    stateLabel,
    topologyHeadline,
    topologySummary,
} from "./cpu_workers_model.mjs";

function ownershipFixture() {
    return {
        available: true,
        reason: null,
        generation: "gen-1",
        circuits: [
            {circuit_id: "c1", parent_node_id: "site:shared", name: "Circuit One", worker_id: 0},
            {circuit_id: "c2", parent_node_id: "site:shared", worker_id: 1},
            {circuit_id: "c3", parent_node_id: "site:single", worker_id: 0},
            {circuit_id: "c4", parent_node_id: "site:single", worker_id: null},
        ],
        nodes: [
            {node_id: "site:shared", parent_node_id: "root", name: "Shared Site", worker_ids: [0, 1], crossing: true},
            {node_id: "site:single", parent_node_id: "root", worker_ids: [0], crossing: false},
            {node_id: "root", parent_node_id: null, name: "Root", worker_ids: [0, 1], crossing: true},
        ],
        crossings: ["root", "site:shared"],
        unowned_circuits: 1,
    };
}

test("normalizes an absent payload to explicit unavailable sections", () => {
    const data = normalizeCpuWorkers(null);
    assert.equal(data.state, "unavailable");
    assert.equal(data.cpuUsage.available, false);
    assert.deepEqual(data.cpuUsage.rows, []);
    assert.equal(data.topology.available, false);
    assert.equal(data.workers.available, false);
    assert.deepEqual(data.workers.rows, []);
    assert.equal(data.ownership.available, false);
    assert.equal(data.ownership.circuits, null);
    assert.equal(data.ownership.nodes, null);
    assert.equal(data.ownership.crossings, null);
    assert.equal(data.moves.available, false);
    assert.equal(data.moves.reason, "no_history_source_v1");
    assert.equal(data.moves.items, null);
    assert.equal(stateLabel(data.state), "Unavailable");
});

test("CPU rows are keyed by cpu_id, never by vector index", () => {
    const data = normalizeCpuWorkers({
        state: "partial",
        cpu_usage: {
            available: true,
            per_cpu: [
                {cpu_id: 5, percent: 0},
                {cpu_id: null, percent: 12},
            ],
            num_cpus: 2,
        },
        cpu_topology: {available: false, reason: "topology_pending"},
        workers: {available: false, reason: "bridge_report_unavailable"},
        ownership: {available: false, reason: "owner_map_unavailable"},
        moves: {available: false, reason: "no_history_source_v1"},
    });
    assert.equal(data.cpuUsage.rows[0].label, "CPU 5");
    assert.equal(data.cpuUsage.rows[0].percent, 0);
    assert.equal(data.cpuUsage.rows[1].label, "CPU ?");
    assert.equal(data.cpuUsage.rows[1].cpuId, null);
    assert.equal(data.cpuUsage.rows[0].key, "cpu-5");
    assert.equal(data.cpuUsage.rows[1].key, "index-1");
});

test("worker metrics keep unsupported distinct from zero", () => {
    assert.equal(formatMetric(null, "unsupported"), "unsupported");
    assert.equal(formatMetric(0, "available"), "0");
    assert.equal(formatMetric(null, null), "unsupported");
    assert.equal(formatCount(0), "0");
    assert.equal(formatCount(null), "—");
    assert.equal(formatQueueIds([]), "none");
    assert.equal(formatQueueIds(null), "—");
    const data = normalizeCpuWorkers({
        workers: {
            available: true,
            counter_support: "unsupported",
            rows: [{
                worker_id: 0,
                lcore_id: 0,
                numa_node: null,
                rx_queue_ids: [],
                tx_queue_id: null,
                shared_tx_queue: null,
                circuit_count: 0,
                solely_owned_node_count: null,
                counters: null,
                throughput: null,
                throughput_support: "unsupported",
                drops: null,
                drops_support: "unsupported",
            }],
        },
    });
    const row = data.workers.rows[0];
    assert.equal(row.lcoreId, 0, "CPU id 0 is a real value");
    assert.equal(row.numaNode, null);
    assert.equal(row.circuitCount, 0, "a present map can compute zero");
    assert.equal(row.solelyOwnedNodeCount, null, "absent owner map stays null");
    assert.equal(row.throughputSupport, "unsupported");
    assert.equal(formatMetric(row.throughput, row.throughputSupport), "unsupported");
    assert.equal(formatMetric(row.drops, row.dropsSupport), "unsupported");
});

test("a site sharded across workers is explicit, with chips and a crossings entry", () => {
    const ownership = normalizeCpuWorkers({ownership: ownershipFixture()}).ownership;
    assert.equal(ownership.available, true);
    assert.equal(ownership.crossings.includes("site:shared"), true);
    assert.equal(ownership.unownedCircuits, 1);
    assert.equal(ownership.circuits.find((c) => c.circuitId === "c4").workerId, null);

    const shared = ownership.nodes.find((node) => node.nodeId === "site:shared");
    const single = ownership.nodes.find((node) => node.nodeId === "site:single");
    assert.equal(shardedSiteLabel(shared), "sharded across 2 workers");
    assert.equal(shardedSiteLabel(single), null);
    assert.deepEqual(shared.workerIds, [0, 1]);
    assert.equal(shared.crossing, true);
    assert.equal(countShardedSites(ownership.nodes), 2, "root + site:shared");
    assert.equal(shardedSummary(ownership), "2");
    assert.equal(siteName(shared), "Shared Site");
    assert.equal(siteName(single), "Unknown site name", "an absent name never falls back to the id");
    assert.equal(siteName({name: "   ", nodeId: "site:blank"}), "Unknown site name");
    assert.equal(circuitName({name: "", circuitId: "c:1"}), "Unknown circuit");
    assert.equal(circuitName(ownership.circuits.find((c) => c.circuitId === "c1")), "Circuit One");
    assert.equal(circuitName(ownership.circuits.find((c) => c.circuitId === "c2")), "Unknown circuit");
});

test("rendered ownership uses names and synthetic DOM ids, never stable ids", () => {
    const ownership = normalizeCpuWorkers({ownership: ownershipFixture()}).ownership;
    const view = ownershipView(ownership);
    assert.equal(view.available, true);
    assert.ok(view.nodes.every((node) => node.domId.startsWith("cpu-node-")));
    assert.equal(new Set(view.nodes.map((node) => node.domId)).size, view.nodes.length);
    assert.deepEqual(
        [...nodeDomIds(ownership.nodes).entries()],
        [...nodeDomIds([...ownership.nodes]).entries()],
        "synthetic ids are stable for the same node order",
    );

    const rendered = JSON.stringify({
        nodes: view.nodes.map((node) => ({name: node.name, domId: node.domId, title: node.name})),
        crossings: view.crossings.map((crossing) => ({name: crossing.name, domId: crossing.domId})),
        circuits: view.circuits.map((circuit) => ({name: circuit.name, siteName: circuit.siteName})),
    });
    for (const stableId of ["site:shared", "site:single", "root", "c1", "c2", "c3", "c4"]) {
        assert.equal(
            rendered.includes(stableId),
            false,
            `stable id ${stableId} must never reach rendered text/title/aria/href`,
        );
    }
    assert.equal(siteName(view.nodes.find((node) => node.nodeId === "site:single")), "Unknown site name");
    assert.equal(
        view.circuits.find((circuit) => circuit.name === "Unknown circuit" && circuit.workerId === null).siteName,
        "Unknown site name",
    );
    assert.equal(view.circuits.find((circuit) => circuit.name === "Circuit One").siteName, "Shared Site");
});

test("sharded count is unavailable without an owner map, never zero", () => {
    const ownership = normalizeCpuWorkers({
        ownership: {available: false, reason: "owner_map_unavailable"},
    }).ownership;
    assert.equal(ownership.available, false);
    assert.equal(shardedSummary(ownership), "owner_map_unavailable");
    assert.notEqual(shardedSummary(ownership), "0");
    assert.equal(countShardedSites(null), null);
});

test("ownership tree is root-first and cycle-safe", () => {
    const ownership = normalizeCpuWorkers({ownership: ownershipFixture()}).ownership;
    const tree = buildOwnershipTree(ownership.nodes);
    const flat = flattenOwnershipTree(tree);
    assert.equal(flat[0].nodeId, "root");
    assert.equal(flat[0].depth, 0);
    assert.equal(flat.some((node) => node.nodeId === "site:shared"), true);
    assert.equal(flat.find((node) => node.nodeId === "site:shared").depth, 1);

    const cyclic = buildOwnershipTree([
        {nodeId: "a", parentNodeId: "b", workerIds: [0], crossing: false},
        {nodeId: "b", parentNodeId: "a", workerIds: [1], crossing: false},
    ]);
    const cyclicFlat = flattenOwnershipTree(cyclic);
    assert.deepEqual(cyclicFlat.map((node) => node.nodeId).sort(), ["a", "b"]);
    assert.equal(new Set(cyclicFlat.map((node) => node.nodeId)).size, 2);
});

test("an available section with missing arrays is unavailable, not empty", () => {
    const ownership = normalizeCpuWorkers({
        ownership: {available: true},
    }).ownership;
    assert.equal(ownership.available, false);
    assert.equal(ownership.reason, "ownership_payload_incomplete");
    assert.equal(ownership.circuits, null);
    assert.equal(ownership.nodes, null);
    assert.equal(ownership.crossings, null);
    assert.equal(shardedSummary(ownership), "ownership_payload_incomplete");

    const moves = normalizeCpuWorkers({
        moves: {available: true},
    }).moves;
    assert.equal(moves.available, false);
    assert.equal(moves.reason, "moves_payload_incomplete");
    assert.equal(moves.items, null);
    assert.equal(movesSummary(moves), "moves_payload_incomplete");
});

function signatureBase() {
    return {
        state: "partial",
        unavailable_reasons: ["moves_unavailable"],
        generated_at_unix: 100,
        cpu_usage: {available: true, num_cpus: 2, per_cpu: [{cpu_id: 0, percent: 10}]},
        cpu_topology: {available: true, source: "exclusion_disabled"},
        workers: {available: true, rows: []},
        ownership: {available: true, nodes: [], circuits: [], crossings: []},
        moves: {available: false, reason: "no_history_source_v1", source: null, since_unix: 100, items: null},
    };
}

test("per-section refresh guard ignores timestamps and isolates changes", () => {
    const base = signatureBase();
    const baseSignatures = sectionSignatures(base);
    const timestampOnly = {
        ...base,
        generated_at_unix: 200,
        moves: {...base.moves, since_unix: 200},
    };
    assert.deepEqual(sectionSignatures(timestampOnly), baseSignatures);

    const cpuChanged = sectionSignatures({
        ...base,
        cpu_usage: {...base.cpu_usage, per_cpu: [{cpu_id: 0, percent: 90}]},
    });
    assert.notEqual(cpuChanged.cpuUsage, baseSignatures.cpuUsage);
    assert.equal(cpuChanged.workers, baseSignatures.workers);
    assert.equal(cpuChanged.tree, baseSignatures.tree);
    assert.equal(cpuChanged.moves, baseSignatures.moves);
    assert.equal(cpuChanged.state, baseSignatures.state);

    const ownershipChanged = sectionSignatures({
        ...base,
        ownership: {...base.ownership, crossings: ["site:shared"]},
    });
    assert.notEqual(ownershipChanged.workers, baseSignatures.workers);
    assert.notEqual(ownershipChanged.tree, baseSignatures.tree);
    assert.equal(ownershipChanged.cpuUsage, baseSignatures.cpuUsage);

    const topologyChanged = sectionSignatures({
        ...base,
        cpu_topology: {...base.cpu_topology, source: "core_type_sysfs"},
    });
    assert.notEqual(topologyChanged.topology, baseSignatures.topology);
    assert.notEqual(topologyChanged.tree, baseSignatures.tree);
    assert.equal(topologyChanged.cpuUsage, baseSignatures.cpuUsage);

    const movesChanged = sectionSignatures({
        ...base,
        moves: {...base.moves, reason: "available"},
    });
    assert.notEqual(movesChanged.moves, baseSignatures.moves);
    assert.equal(movesChanged.cpuUsage, baseSignatures.cpuUsage);
});

test("CPU heat scale is neutral for unknown and green/amber/red for usage", () => {
    assert.equal(cpuHeatColor(null), null);
    assert.equal(cpuHeatColor(undefined), null);
    assert.match(cpuHeatColor(0), /^rgba\(25, 135, 84/);
    assert.match(cpuHeatColor(60), /^rgba\(255, 193, 7/);
    assert.match(cpuHeatColor(95), /^rgba\(220, 53, 69/);
});

test("the raw bus shape is a protocol error, the assembled shape is accepted", () => {
    const assembled = {
        state: "partial",
        unavailable_reasons: ["no_history_source_v1"],
        generated_at_unix: 1,
        cpu_usage: {available: true},
        cpu_topology: {available: true},
        bridge: {available: true},
        workers: {available: true},
        ownership: {available: false},
        moves: {available: false},
    };
    assert.equal(isAssembledCpuWorkersPayload(assembled), true);
    const rawBus = {
        schema: 1,
        generated_at_unix: 1,
        bridge: {available: true},
        workers: {available: true},
        ownership: {available: true},
        moves: {available: false},
    };
    assert.equal(isAssembledCpuWorkersPayload(rawBus), false);
    assert.equal(isAssembledCpuWorkersPayload(null), false);
    assert.equal(isAssembledCpuWorkersPayload({...assembled, cpu_usage: null}), false);
    assert.equal(isAssembledCpuWorkersPayload({...assembled, state: "bogus"}), false);
    assert.equal(stateLabel("protocol_error"), "Protocol error");
});

test("topology/moves report honest-empty reasons", () => {
    const data = normalizeCpuWorkers({
        cpu_topology: {available: false, reason: "topology_pending"},
        moves: {available: false, reason: "no_history_source_v1"},
    });
    assert.equal(topologySummary(data.topology), "topology_pending");
    assert.equal(movesSummary(data.moves), "no_history_source_v1");
    assert.equal(movesSummary({available: true, items: []}), "none observed");
});

test("reason codes are humanized and topology has a compact headline", () => {
    assert.equal(reasonText("owner_map_unavailable"), "Ownership map is not available yet");
    assert.equal(reasonText("no_history_source_v1"), "Move history is not available in v1");
    assert.equal(reasonText("some_unknown_code"), "some unknown code");
    assert.equal(topologyHeadline({
        available: true,
        source: "fallback_all_possible",
        possibility: null,
        possible: [0, 1, 2],
        shaping: [0, 1, 2],
        hasHybridSplit: false,
    }), "All 3 CPUs (fallback)");
    assert.equal(topologyHeadline({
        available: true,
        source: "core_type_sysfs",
        possible: [0, 1, 2, 3],
        shaping: [0, 1],
        hasHybridSplit: true,
    }), "Hybrid P/E cores");
    assert.equal(topologyHeadline({available: false, reason: "cpu_topology_unavailable"}), "Unavailable");
});

test("moves-only expected absence keeps the page healthy, not partial", () => {
    const data = normalizeCpuWorkers({
        state: "partial",
        unavailable_reasons: ["no_history_source_v1"],
        cpu_usage: {available: true, per_cpu: [], num_cpus: 0},
        cpu_topology: {available: true},
        bridge: {available: true},
        workers: {available: true, rows: []},
        ownership: {available: true, nodes: [], circuits: [], crossings: []},
        moves: {available: false, reason: "no_history_source_v1"},
    });
    assert.equal(data.state, "ok");
    assert.deepEqual(data.unavailableReasons, []);
    assert.equal(data.moves.available, false);
});

test("ownership siblings are sorted by name, unnamed last", () => {
    const view = ownershipView({
        available: true,
        nodes: [
            {nodeId: "z", name: "Zeta", parentNodeId: null, workerIds: [0]},
            {nodeId: "a", name: "Alpha", parentNodeId: null, workerIds: [0]},
            {nodeId: "u", name: null, parentNodeId: null, workerIds: [0]},
        ],
        circuits: [],
        crossings: [],
        unownedCircuits: 0,
    });
    assert.deepEqual(view.tree.map((node) => node.name), ["Alpha", "Zeta", "Unknown site name"]);
});

test("per-site sharded counts only mark multi-worker sites", () => {
    const counts = shardedSiteWorkerCounts([
        {nodeId: "a", name: "Alpha", workerIds: [0, 1, 2]},
        {nodeId: "b", name: "Beta", workerIds: [1]},
        {nodeId: "c", name: "Gamma", workerIds: []},
        {nodeId: "u", name: null, workerIds: [0, 1]},
    ]);
    assert.equal(counts.get("Alpha"), 3);
    assert.equal(counts.has("Beta"), false);
    assert.equal(counts.has("Gamma"), false);
    assert.equal(counts.size, 1);
});
