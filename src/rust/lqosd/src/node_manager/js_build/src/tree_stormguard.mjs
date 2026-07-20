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
    if (!settings) return "Settings are not available yet.";
    const multiplier = (value) => value === null ? "—" : `×${value.toFixed(2)}`;
    const parts = [
        `increase ${multiplier(settings.increaseMultiplier)} (fast ${multiplier(settings.increaseFastMultiplier)})`,
        `decrease ${multiplier(settings.decreaseMultiplier)} (fast ${multiplier(settings.decreaseFastMultiplier)})`,
    ];
    if (runtime.strategy === "delay_probe" || runtime.strategy === "delay_probe_active") {
        parts.push(
            `delay ${settings.delayThresholdMs?.toFixed(1) ?? "—"} ms or ${multiplier(settings.delayThresholdRatio)} baseline`,
            `probe ${settings.probeIntervalSeconds?.toFixed(1) ?? "—"} s`,
            `passive RTT minimum ${settings.minThroughputMbpsForRtt?.toFixed(2) ?? "—"} Mbps`,
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
        message = `${normalizedSite} is managed by StormGuard.`;
    } else if (managed) {
        message = `${normalizedSite} is managed by StormGuard, but directional diagnostics are not available yet.`;
    } else {
        message = `${normalizedSite || "The selected node"} is not managed by StormGuard. Select a watched site to see directional diagnostics.`;
    }
    return {
        entry,
        managed,
        diagnosticsPending: managed && entry === null,
        message,
    };
}

function trendLabel(points, field) {
    const values = points
        .map((point) => finiteOrNull(point[field]))
        .filter((value) => value !== null);
    if (values.length < 2) return "not enough data for a trend";
    const delta = values.at(-1) - values[0];
    if (Math.abs(delta) < 0.01) return "steady";
    return delta > 0 ? "increasing" : "decreasing";
}

export function summarizeStormguardHistory(points, directionName) {
    if (!Array.isArray(points) || points.length === 0) {
        return `No ${directionName} graph history is available yet.`;
    }
    const latest = points.at(-1);
    const value = (number, unit) => {
        const numeric = finiteOrNull(number);
        return numeric === null ? "unavailable" : `${numeric.toFixed(1)} ${unit}`;
    };
    const actions = points
        .filter((point) => point.marker)
        .slice(-3)
        .map((point) => `${point.marker.action || "action"} ${point.marker.outcome || "unknown"}`);
    return [
        `Five-minute ${directionName} history.`,
        `Latest queue limit ${value(latest.queueMbps, "Mbps")}; throughput ${value(latest.throughputMbps, "Mbps")}; effective RTT ${value(latest.effectiveRttMs, "ms")}; cooldown ${value(latest.cooldownSeconds, "seconds")}.`,
        `Queue limit trend: ${trendLabel(points, "queueMbps")}.`,
        actions.length > 0 ? `Recent actions: ${actions.join(", ")}.` : "No actions are present in this history window.",
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
