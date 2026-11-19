import assert from "node:assert/strict";
import test from "node:test";
import {
    StormguardHistory,
    formatStormguardSettings,
    normalizeStormguardDebug,
    normalizeStormguardRuntime,
    shouldShowStormguardTab,
    stormguardNodeContext,
    summarizeStormguardHistory,
} from "./tree_stormguard.mjs";
import {
    clearCatalogs,
    EN,
    ES,
    installCatalogs,
    installKeyEchoCatalogs,
    installTemplatesOnly,
} from "../i18n/test_catalogs.mjs";

test.beforeEach(() => installCatalogs());
test.afterEach(clearCatalogs);

test("tab visibility includes configured, cleanup, and degraded runtime states", () => {
    assert.equal(shouldShowStormguardTab(normalizeStormguardRuntime({configured_enabled: true})), true);
    assert.equal(shouldShowStormguardTab(normalizeStormguardRuntime({cleanup_pending: true})), true);
    assert.equal(shouldShowStormguardTab(normalizeStormguardRuntime({phase: "degraded"})), true);
    assert.equal(shouldShowStormguardTab(normalizeStormguardRuntime({phase: "disabled"})), false);
});

test("runtime settings normalize and format strategy inputs", () => {
    const runtime = normalizeStormguardRuntime({
        strategy: "delay_probe",
        settings: {
            increase_fast_multiplier: 1.3,
            increase_multiplier: 1.15,
            decrease_multiplier: 0.95,
            decrease_fast_multiplier: 0.88,
            delay_threshold_ms: 40,
            delay_threshold_ratio: 1.1,
            probe_interval_seconds: 10,
            min_throughput_mbps_for_rtt: 0.05,
        },
    });
    const formatted = formatStormguardSettings(runtime);
    assert.match(formatted, /increase ×1.15/);
    assert.match(formatted, /delay 40.0 ms/);
    assert.match(formatted, /passive RTT minimum 0.05 Mbps/);
});

test("debug payload normalization retains node context and direction data", () => {
    const sites = normalizeStormguardDebug([{
        site: "North",
        download: {queue_mbps: 50},
        upload: {queue_mbps: 20},
        active_ping_target: "1.1.1.1",
        passive_rtt_flow_counts: [4, 2],
    }, {site: ""}, null]);
    assert.equal(sites.size, 1);
    assert.equal(sites.get("North").download.queue_mbps, 50);
    assert.deepEqual(sites.get("North").passiveRttFlowCounts, [4, 2]);
});

test("history stays bounded and de-duplicates repeated action markers", () => {
    const history = new StormguardHistory(2);
    const sample = {
        queue_mbps: 50,
        last_attempt_unix_ms: 1000,
        last_attempt_action: "decrease",
        last_attempt_outcome: "applied",
    };
    history.push("North", "download", sample, 1);
    history.push("North", "download", sample, 2);
    history.push("North", "download", {...sample, last_attempt_unix_ms: 2000}, 3);
    const points = history.points("North", "download");
    assert.equal(points.length, 2);
    assert.equal(points[0].marker, null);
    assert.equal(points[1].marker.outcome, "applied");
});

test("history removes samples older than five minutes even with irregular updates", () => {
    const history = new StormguardHistory(300);
    history.push("North", "download", {queue_mbps: 50}, 0);
    history.push("North", "download", {queue_mbps: 49}, 1000);
    history.push("North", "download", {queue_mbps: 48}, 300001);
    assert.deepEqual(
        history.points("North", "download").map((point) => point.timestamp),
        [1000, 300001],
    );
});

test("action markers retain their application time and expired history prunes on read", () => {
    const history = new StormguardHistory(300);
    history.push("North", "download", {
        queue_mbps: 50,
        last_attempt_unix_ms: 1000,
        last_attempt_action: "decrease",
        last_attempt_outcome: "applied",
    }, 2000);
    assert.equal(history.points("North", "download")[0].marker.timestamp, 1000);
    assert.deepEqual(history.points("North", "download", 302001), []);
});

test("an old action marker expires independently of its current sample", () => {
    const history = new StormguardHistory(300);
    history.push("North", "download", {
        queue_mbps: 50,
        last_attempt_unix_ms: 1000,
        last_attempt_action: "decrease",
        last_attempt_outcome: "applied",
    }, 300000);
    const points = history.points("North", "download", 301001);
    assert.equal(points.length, 1);
    assert.equal(points[0].marker, null);
});

test("node context explains managed and unmanaged selections", () => {
    const debug = normalizeStormguardDebug([{site: "North", download: {}, upload: {}}]);
    assert.equal(stormguardNodeContext("North", debug, new Set()).managed, true);
    const retained = stormguardNodeContext("South", new Map(), new Set(["South"]));
    assert.equal(retained.managed, true);
    assert.equal(retained.diagnosticsPending, true);
    assert.match(retained.message, /not available yet/);
    const unmanaged = stormguardNodeContext("Branch", new Map(), new Set());
    assert.equal(unmanaged.managed, false);
    assert.match(unmanaged.message, /not managed by StormGuard/);
});

test("accessible history summary reports latest values, trend, and actions", () => {
    const summary = summarizeStormguardHistory([
        {queueMbps: 50, throughputMbps: 40, effectiveRttMs: 20, cooldownSeconds: 0, marker: null},
        {queueMbps: 45, throughputMbps: 39, effectiveRttMs: 25, cooldownSeconds: 3, marker: {action: "decrease", outcome: "applied"}},
    ], "download");
    assert.match(summary, /Download: latest queue limit 45.0 Mbps/);
    assert.match(summary, /Queue limit trend: decreasing/);
    assert.match(summary, /Result of decrease: applied/);
});

test("invalid history inputs do not create a series", () => {
    const history = new StormguardHistory();
    history.push("North", "sideways", {}, 1);
    assert.deepEqual(history.points("North", "sideways"), []);
});

test("history drops sites that are no longer selected", () => {
    const history = new StormguardHistory();
    history.push("North", "download", {queue_mbps: 50}, 1);
    history.push("South", "download", {queue_mbps: 40}, 1);
    history.retainSites(new Set(["South"]));
    assert.deepEqual(history.points("North", "download"), []);
    assert.equal(history.points("South", "download").length, 1);
});

test("every StormGuard code maps to catalog keys both locales define", () => {
    installKeyEchoCatalogs();
    const defined = (text) => /^(?:tree\.stormguard|common\.table)\./.test(text)
        && Object.prototype.hasOwnProperty.call(EN.messages, text)
        && Object.prototype.hasOwnProperty.call(ES.messages, text);

    // Settings: one key per clause.
    const probe = normalizeStormguardRuntime({
        strategy: "delay_probe",
        settings: {
            increase_fast_multiplier: 1.3,
            increase_multiplier: 1.15,
            decrease_multiplier: 0.95,
            decrease_fast_multiplier: 0.88,
            delay_threshold_ms: 40,
            delay_threshold_ratio: 1.1,
            probe_interval_seconds: 10,
            min_throughput_mbps_for_rtt: 0.05,
        },
    });
    const clauses = formatStormguardSettings(probe).split("; ");
    assert.equal(clauses.length, 5);
    for (const clause of clauses) assert.ok(defined(clause), `settings clause "${clause}"`);
    assert.ok(defined(formatStormguardSettings(normalizeStormguardRuntime({}))));

    // Node context: managed, diagnostics pending, unmanaged.
    const debug = normalizeStormguardDebug([{site: "North"}]);
    assert.ok(defined(stormguardNodeContext("North", debug, new Set()).message));
    assert.ok(defined(stormguardNodeContext("South", new Map(), new Set(["South"])).message));
    assert.ok(defined(stormguardNodeContext("Branch", new Map(), new Set()).message));

    // History: intro, latest, trend, actions and no-actions are one key each;
    // the direction, trend, action and outcome labels are keys inside them.
    const summary = summarizeStormguardHistory([
        {queueMbps: 50, throughputMbps: 40, effectiveRttMs: 20, cooldownSeconds: 0, marker: null},
    ], "download");
    for (const key of summary.split(" ")) {
        assert.match(key, /^tree\.stormguard\./, `history part "${key}"`);
        assert.ok(Object.prototype.hasOwnProperty.call(EN.messages, key), `en.json is missing "${key}"`);
        assert.ok(Object.prototype.hasOwnProperty.call(ES.messages, key), `es.json is missing "${key}"`);
    }
    assert.ok(defined(summarizeStormguardHistory([], "upload")));
});

test("labels that reach the screen as parameters are catalog keys too", () => {
    installTemplatesOnly([
        "tree.stormguard.history_latest",
        "tree.stormguard.history_trend",
        "tree.stormguard.history_actions",
        "tree.stormguard.history_action_entry",
        "tree.stormguard.not_managed",
    ]);
    const summary = summarizeStormguardHistory([
        {queueMbps: 50, throughputMbps: 40, effectiveRttMs: 20, cooldownSeconds: 0, marker: null},
        {queueMbps: 45, throughputMbps: 39, effectiveRttMs: 25, cooldownSeconds: 3,
            marker: {action: "decrease_fast", outcome: "dry_run"}},
        {queueMbps: 45, throughputMbps: 39, effectiveRttMs: 25, cooldownSeconds: 3,
            marker: {action: "", outcome: ""}},
    ], "upload");
    assert.match(summary, /common\.table\.upload/, "the direction label is a key");
    assert.match(summary, /45\.0/, "the reading still interpolates");
    assert.match(summary, /tree\.stormguard\.trend\.decreasing/);
    assert.match(summary, /tree\.stormguard\.action\.decrease_fast/);
    assert.match(summary, /tree\.stormguard\.outcome\.dry_run/);
    assert.match(summary, /tree\.stormguard\.action_fallback/, "an empty action keeps a word");
    assert.match(summary, /tree\.stormguard\.outcome_fallback/, "an empty outcome keeps a word");
    assert.match(
        stormguardNodeContext("", new Map(), new Set()).message,
        /tree\.stormguard\.selected_node/,
        "the no-selection subject is a key, not an English fallback",
    );

    installTemplatesOnly(["tree.stormguard.history_latest"]);
    const missing = summarizeStormguardHistory([
        {queueMbps: null, throughputMbps: null, effectiveRttMs: null, cooldownSeconds: null, marker: null},
    ], "download");
    assert.match(missing, /tree\.stormguard\.value_unavailable/);
});

test("an action or outcome code this module does not know stays visible", () => {
    const summary = summarizeStormguardHistory([
        {queueMbps: 50, throughputMbps: 40, effectiveRttMs: 20, cooldownSeconds: 0,
            marker: {action: "surge", outcome: "superseded"}},
    ], "download");
    assert.match(summary, /Result of surge: superseded/);
});

test("StormGuard prose follows the served locale and keeps the wire codes", () => {
    installCatalogs(ES);
    const runtime = normalizeStormguardRuntime({
        strategy: "delay_probe",
        settings: {
            increase_fast_multiplier: 1.3,
            increase_multiplier: 1.15,
            decrease_multiplier: 0.95,
            decrease_fast_multiplier: 0.88,
            delay_threshold_ms: 40,
            delay_threshold_ratio: 1.1,
            probe_interval_seconds: 10,
            min_throughput_mbps_for_rtt: 0.05,
        },
    });
    const formatted = formatStormguardSettings(runtime);
    assert.match(formatted, /incremento ×1,15/);
    assert.match(formatted, /retardo 40,0\u00a0ms/);
    assert.match(formatted, /tasa de transferencia mínima para medir el RTT pasivo: 0,05\u00a0Mbps/);

    const summary = summarizeStormguardHistory([
        {queueMbps: 50, throughputMbps: 40, effectiveRttMs: 20, cooldownSeconds: 0, marker: null},
        {queueMbps: 45, throughputMbps: 39, effectiveRttMs: 25, cooldownSeconds: 3,
            marker: {action: "decrease_fast", outcome: "applied"}},
    ], "download");
    assert.match(summary, /Historial de cinco minutos: Descarga\./);
    assert.match(summary, /Tendencia del límite de cola: en descenso\./);
    assert.match(summary, /Acción: reducción rápida; resultado: aplicado/);
    assert.equal(runtime.strategy, "delay_probe", "the strategy code is never translated");
});
