import {formatNumber} from "./i18n/format.js";
import {t} from "./i18n/index.js";

const HISTORY_SECONDS = 5 * 60;
const HISTORY_WINDOW_MS = HISTORY_SECONDS * 1000;

function finiteOrNull(value) {
    if (value === null || value === undefined || value === "") return null;
    const number = Number(value);
    return Number.isFinite(number) ? number : null;
}

function stringOrEmpty(value) {
    return value === null || value === undefined ? "" : String(value);
}

// A fixed number of decimals in the active locale's notation ("45.0" / "45,0"),
// never grouped: these are single readings, not counts.
function formatFixed(value, digits) {
    return formatNumber(value, {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
        useGrouping: false,
    });
}

// The wire direction stays a code ("download"/"upload"); only the word shown
// comes from the catalog.
function directionLabel(directionName) {
    return directionName === "upload" ? t("common.table.upload") : t("common.table.download");
}

// Action/outcome codes from `lqos_stormguard` (`RecommendationAction::as_str`
// and the attempt outcome strings). The code is never rewritten; an unknown code
// is reported as it arrived so a new server value stays visible.
function actionLabel(code) {
    switch (code) {
    case "increase":
        return t("tree.stormguard.action.increase");
    case "increase_fast":
        return t("tree.stormguard.action.increase_fast");
    case "decrease":
        return t("tree.stormguard.action.decrease");
    case "decrease_fast":
        return t("tree.stormguard.action.decrease_fast");
    default:
        return code || t("tree.stormguard.action_fallback");
    }
}

function outcomeLabel(code) {
    switch (code) {
    case "applied":
        return t("tree.stormguard.outcome.applied");
    case "dry_run":
        return t("tree.stormguard.outcome.dry_run");
    case "failed":
        return t("tree.stormguard.outcome.failed");
    case "skipped":
        return t("tree.stormguard.outcome.skipped");
    default:
        return code || t("tree.stormguard.outcome_fallback");
    }
}

function trendLabel(points, field) {
    const values = points
        .map((point) => finiteOrNull(point[field]))
        .filter((value) => value !== null);
    if (values.length < 2) return t("tree.stormguard.trend.not_enough");
    const delta = values.at(-1) - values[0];
    if (Math.abs(delta) < 0.01) return t("tree.stormguard.trend.steady");
    return delta > 0 ? t("tree.stormguard.trend.increasing") : t("tree.stormguard.trend.decreasing");
}

export function normalizeStormguardRuntime(payload) {
    const value = payload && typeof payload === "object" ? payload : {};
    return {
        configuredEnabled: value.configured_enabled === true,
        mode: stringOrEmpty(value.mode) || "disabled",
        phase: stringOrEmpty(value.phase) || "disabled",
        bakeryReady: value.bakery_ready === true,
        cleanupPending: value.cleanup_pending === true,
        strategy: stringOrEmpty(value.strategy),
        activePingTarget: stringOrEmpty(value.active_ping_target),
        activePingWeight: finiteOrNull(value.active_ping_weight),
        settings: value.settings && typeof value.settings === "object" ? {
            increaseFastMultiplier: finiteOrNull(value.settings.increase_fast_multiplier),
            increaseMultiplier: finiteOrNull(value.settings.increase_multiplier),
            decreaseMultiplier: finiteOrNull(value.settings.decrease_multiplier),
            decreaseFastMultiplier: finiteOrNull(value.settings.decrease_fast_multiplier),
            delayThresholdMs: finiteOrNull(value.settings.delay_threshold_ms),
            delayThresholdRatio: finiteOrNull(value.settings.delay_threshold_ratio),
            probeIntervalSeconds: finiteOrNull(value.settings.probe_interval_seconds),
            minThroughputMbpsForRtt: finiteOrNull(value.settings.min_throughput_mbps_for_rtt),
        } : null,
        message: stringOrEmpty(value.message),
        lastError: stringOrEmpty(value.last_error),
        updatedAtUnixMs: finiteOrNull(value.updated_at_unix_ms),
    };
}

export function formatStormguardSettings(runtime) {
    const settings = runtime?.settings;
    if (!settings) return t("tree.stormguard.settings_unavailable");
    const missing = "—";
    const multiplier = (value) => value === null ? missing : `×${formatFixed(value, 2)}`;
    const decimal = (value, digits) => value === null ? missing : formatFixed(value, digits);
    const parts = [
        t("tree.stormguard.settings_increase", {
            multiplier: multiplier(settings.increaseMultiplier),
            fast: multiplier(settings.increaseFastMultiplier),
        }),
        t("tree.stormguard.settings_decrease", {
            multiplier: multiplier(settings.decreaseMultiplier),
            fast: multiplier(settings.decreaseFastMultiplier),
        }),
    ];
    if (runtime.strategy === "delay_probe" || runtime.strategy === "delay_probe_active") {
        parts.push(
            t("tree.stormguard.settings_delay", {
                ms: decimal(settings.delayThresholdMs, 1),
                ratio: multiplier(settings.delayThresholdRatio),
            }),
            t("tree.stormguard.settings_probe", {
                seconds: decimal(settings.probeIntervalSeconds, 1),
            }),
            t("tree.stormguard.settings_passive_min", {
                mbps: decimal(settings.minThroughputMbpsForRtt, 2),
            }),
        );
    }
    return parts.join("; ");
}

export function shouldShowStormguardTab(runtime) {
    return runtime?.configuredEnabled === true
        || runtime?.cleanupPending === true
        || runtime?.phase === "degraded"
        || runtime?.phase === "cleanup_pending";
}

export function normalizeStormguardDebug(entries) {
    if (!Array.isArray(entries)) return new Map();
    const result = new Map();
    entries.forEach((entry) => {
        const site = stringOrEmpty(entry?.site).trim();
        if (!site) return;
        result.set(site, {
            site,
            download: entry?.download && typeof entry.download === "object" ? entry.download : null,
            upload: entry?.upload && typeof entry.upload === "object" ? entry.upload : null,
            activePingTarget: stringOrEmpty(entry?.active_ping_target),
            activePingWeight: finiteOrNull(entry?.active_ping_weight),
            passiveRttFlowCounts: Array.isArray(entry?.passive_rtt_flow_counts)
                ? [finiteOrNull(entry.passive_rtt_flow_counts[0]) ?? 0, finiteOrNull(entry.passive_rtt_flow_counts[1]) ?? 0]
                : [0, 0],
        });
    });
    return result;
}

export function stormguardNodeContext(siteName, debugBySite, managedSites) {
    const normalizedSite = stringOrEmpty(siteName).trim();
    const entry = debugBySite instanceof Map ? debugBySite.get(normalizedSite) || null : null;
    const managed = entry !== null
        || (managedSites instanceof Set && managedSites.has(normalizedSite));
    let message;
    if (entry !== null) {
        message = t("tree.stormguard.managed", {subject: normalizedSite});
    } else if (managed) {
        message = t("tree.stormguard.managed_pending", {subject: normalizedSite});
    } else {
        message = t("tree.stormguard.not_managed", {
            subject: normalizedSite || t("tree.stormguard.selected_node"),
        });
    }
    return {
        entry,
        managed,
        diagnosticsPending: managed && entry === null,
        message,
    };
}

export function summarizeStormguardHistory(points, directionName) {
    const direction = directionLabel(directionName);
    if (!Array.isArray(points) || points.length === 0) {
        return t("tree.stormguard.no_history_for", {direction});
    }
    const latest = points.at(-1);
    const reading = (number) => {
        const numeric = finiteOrNull(number);
        return numeric === null ? t("tree.stormguard.value_unavailable") : formatFixed(numeric, 1);
    };
    const actions = points
        .filter((point) => point.marker)
        .slice(-3)
        .map((point) => t("tree.stormguard.history_action_entry", {
            action: actionLabel(point.marker.action),
            outcome: outcomeLabel(point.marker.outcome),
        }));
    return [
        t("tree.stormguard.history_intro", {direction}),
        t("tree.stormguard.history_latest", {
            direction,
            limit: reading(latest.queueMbps),
            throughput: reading(latest.throughputMbps),
            rtt: reading(latest.effectiveRttMs),
            cooldown: reading(latest.cooldownSeconds),
        }),
        t("tree.stormguard.history_trend", {trend: trendLabel(points, "queueMbps")}),
        actions.length > 0
            ? t("tree.stormguard.history_actions", {actions: actions.join(", ")})
            : t("tree.stormguard.history_no_actions"),
    ].join(" ");
}

export class StormguardHistory {
    constructor(capacity = HISTORY_SECONDS, windowMs = HISTORY_WINDOW_MS) {
        this.capacity = capacity;
        this.windowMs = windowMs;
        this.seriesByKey = new Map();
        this.lastMarkerByKey = new Map();
    }

    push(site, directionName, direction, timestamp = Date.now()) {
        if (!site || !direction || !["download", "upload"].includes(directionName)) return;
        const key = `${site}\u0000${directionName}`;
        const attemptTimestamp = finiteOrNull(direction.last_attempt_unix_ms);
        const markerKey = attemptTimestamp === null
            ? null
            : `${attemptTimestamp}:${stringOrEmpty(direction.last_attempt_action)}:${stringOrEmpty(direction.last_attempt_outcome)}`;
        const markerIsCurrent = attemptTimestamp !== null
            && attemptTimestamp >= timestamp - this.windowMs;
        const marker = markerIsCurrent && markerKey !== this.lastMarkerByKey.get(key)
            ? {
                timestamp: attemptTimestamp,
                action: stringOrEmpty(direction.last_attempt_action),
                outcome: stringOrEmpty(direction.last_attempt_outcome),
                targetMbps: finiteOrNull(direction.last_attempt_target_mbps),
            }
            : null;
        if (markerKey) this.lastMarkerByKey.set(key, markerKey);

        const points = this.seriesByKey.get(key) || [];
        points.push({
            timestamp,
            queueMbps: finiteOrNull(direction.queue_mbps),
            throughputMbps: finiteOrNull(direction.throughput_mbps),
            minMbps: finiteOrNull(direction.min_mbps),
            maxMbps: finiteOrNull(direction.max_mbps),
            passiveRttMs: finiteOrNull(direction.passive_rtt_ms),
            activeRttMs: finiteOrNull(direction.active_ping_rtt_ms),
            effectiveRttMs: finiteOrNull(direction.rtt),
            baselineRttMs: finiteOrNull(direction.baseline_rtt_ms),
            delayMs: finiteOrNull(direction.delay_ms),
            cooldownSeconds: finiteOrNull(direction.cooldown_remaining_secs),
            decisionScore: finiteOrNull(direction.decision_score),
            marker,
        });
        const cutoff = timestamp - this.windowMs;
        while (points.length > 0 && points[0].timestamp < cutoff) points.shift();
        if (points.length > this.capacity) points.splice(0, points.length - this.capacity);
        this.seriesByKey.set(key, points);
    }

    points(site, directionName, now = null) {
        const key = `${site}\u0000${directionName}`;
        const points = this.seriesByKey.get(key) || [];
        const currentTime = finiteOrNull(now);
        if (currentTime !== null) {
            const cutoff = currentTime - this.windowMs;
            while (points.length > 0 && points[0].timestamp < cutoff) points.shift();
            points.forEach((point) => {
                if (point.marker && point.marker.timestamp < cutoff) point.marker = null;
            });
        }
        return points;
    }

    retainSites(siteNames) {
        const retained = siteNames instanceof Set ? siteNames : new Set();
        for (const key of this.seriesByKey.keys()) {
            if (!retained.has(key.split("\u0000", 1)[0])) this.seriesByKey.delete(key);
        }
        for (const key of this.lastMarkerByKey.keys()) {
            if (!retained.has(key.split("\u0000", 1)[0])) this.lastMarkerByKey.delete(key);
        }
    }
}
