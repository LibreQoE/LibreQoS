import {loadConfig} from "../config/config_helper";

const CHANGE_RECENT_WINDOW_SECONDS = 300;

const listeners = new Set();
const store = {
    configRequested: false,
    config: null,
    configError: false,
    lastStatusPayload: null,
    lastDebugPayload: null,
    statusRows: [],
    debugEntries: [],
    selectedSite: null,
};

function num(value) {
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
}

function text(value) {
    return (value ?? "").toString();
}

function titleCase(value) {
    const s = text(value).trim();
    if (!s) return "";
    return s.charAt(0).toUpperCase() + s.slice(1);
}

export function formatStormguardMbps(value) {
    const n = num(value);
    if (n === null) return "—";
    return n.toLocaleString(undefined, {maximumFractionDigits: 2});
}

export function formatStormguardPercent(value) {
    const n = num(value);
    if (n === null) return "—";
    return `${(n * 100).toFixed(2)}%`;
}

export function formatStormguardMs(value) {
    const n = num(value);
    if (n === null) return "—";
    return `${n.toFixed(2)} ms`;
}

export function formatStormguardAgeSeconds(value) {
    const n = num(value);
    if (n === null) return "—";
    if (n < 60) return `${Math.round(n)}s`;
    if (n < 3600) return `${Math.floor(n / 60)}m ${Math.round(n % 60)}s`;
    return `${Math.floor(n / 3600)}h ${Math.floor((n % 3600) / 60)}m`;
}

export function directionSummary(direction) {
    if (!direction) {
        return {
            label: "No Data",
            className: "bg-light text-secondary border",
            reason: "No live metrics yet",
        };
    }

    const cooldown = num(direction.cooldown_remaining_secs) ?? 0;
    const queue = num(direction.queue_mbps);
    const min = num(direction.min_mbps);
    const max = num(direction.max_mbps);
    const action = text(direction.last_action).trim().toLowerCase();
    const state = text(direction.state).trim().toLowerCase();
    const delay = num(direction.delay_ms);
    const retrans = num(direction.retrans_ma) ?? num(direction.retrans);

    if (cooldown > 0.05) {
        return {
            label: "Cooling Down",
            className: "bg-warning-subtle text-warning border border-warning-subtle",
            reason: `${cooldown.toFixed(1)}s remaining`,
        };
    }

    if (action.includes("decrease")) {
        return {
            label: "Reducing",
            className: "bg-danger-subtle text-danger border border-danger-subtle",
            reason: "StormGuard most recently lowered this queue",
        };
    }

    if (action.includes("increase")) {
        return {
            label: "Recovering",
            className: "bg-success-subtle text-success border border-success-subtle",
            reason: "StormGuard most recently raised this queue",
        };
    }

    if (queue !== null && min !== null && queue <= min && direction.can_decrease === false) {
        return {
            label: "At Floor",
            className: "bg-secondary-subtle text-secondary border border-secondary-subtle",
            reason: "Queue is already at its configured minimum",
        };
    }

    if (queue !== null && max !== null && queue >= max && direction.can_increase === false) {
        return {
            label: "At Ceiling",
            className: "bg-primary-subtle text-primary border border-primary-subtle",
            reason: "Queue is already at its configured maximum",
        };
    }

    if (delay !== null && delay > 5) {
        return {
            label: "Congestion Signal",
            className: "bg-danger-subtle text-danger border border-danger-subtle",
            reason: `Standing delay is ${delay.toFixed(2)} ms above baseline`,
        };
    }

    if (retrans !== null && retrans >= 0.01) {
        return {
            label: "Loss Signal",
            className: "bg-warning-subtle text-warning border border-warning-subtle",
            reason: `Retransmits are at ${formatStormguardPercent(retrans)}`,
        };
    }

    if (state === "warmup") {
        return {
            label: "Warming Up",
            className: "bg-info-subtle text-info border border-info-subtle",
            reason: "StormGuard is collecting baseline samples",
        };
    }

    return {
        label: "Holding",
        className: "bg-light text-secondary border",
        reason: "Monitoring conditions without changing the queue",
    };
}

export function directionReason(direction) {
    if (!direction) return "No live metrics yet.";
    const cooldown = num(direction.cooldown_remaining_secs) ?? 0;
    if (cooldown > 0.05) {
        return `Cooldown active for ${cooldown.toFixed(1)}s.`;
    }

    const delay = num(direction.delay_ms);
    if (delay !== null && delay > 0.5) {
        return `Standing delay is ${delay.toFixed(2)} ms above the learned baseline.`;
    }

    const retrans = num(direction.retrans_ma) ?? num(direction.retrans);
    if (retrans !== null && retrans >= 0.005) {
        return `Retransmits are ${formatStormguardPercent(retrans)}.`;
    }

    const queue = num(direction.queue_mbps);
    const min = num(direction.min_mbps);
    const max = num(direction.max_mbps);
    if (queue !== null && min !== null && queue <= min && direction.can_decrease === false) {
        return "The queue is already at the configured minimum.";
    }
    if (queue !== null && max !== null && queue >= max && direction.can_increase === false) {
        return "The queue is already at the configured maximum.";
    }

    const throughput = num(direction.throughput_ma_mbps) ?? num(direction.throughput_mbps);
    if (throughput !== null && max !== null && throughput < max * 0.2) {
        return "Load is low, so StormGuard is holding steady.";
    }

    return "StormGuard is monitoring conditions and waiting for a clearer signal.";
}

function computeAttentionScore(site) {
    let score = 0;
    if (site.changedRecently) score += 100;
    if (site.inCooldown) score += 80;
    if (site.downloadSummary?.label === "Congestion Signal" || site.uploadSummary?.label === "Congestion Signal") {
        score += 60;
    }
    if (site.downloadSummary?.label === "Reducing" || site.uploadSummary?.label === "Reducing") {
        score += 50;
    }
    if (site.downloadSummary?.label === "Recovering" || site.uploadSummary?.label === "Recovering") {
        score += 40;
    }
    if (site.downloadSummary?.label === "At Floor" || site.uploadSummary?.label === "At Floor") {
        score += 20;
    }
    if (site.downloadSummary?.label === "At Ceiling" || site.uploadSummary?.label === "At Ceiling") {
        score += 10;
    }
    return score;
}

function changedRecently(direction) {
    const age = num(direction?.last_action_age_secs);
    return age !== null && age <= CHANGE_RECENT_WINDOW_SECONDS;
}

function normalizeStatusRows(rows) {
    if (!Array.isArray(rows)) return [];
    return rows.map((row) => ({
        site: text(row?.[0]).trim(),
        currentDownMbps: num(row?.[1]),
        currentUpMbps: num(row?.[2]),
    })).filter((row) => row.site.length > 0);
}

function normalizeDebugEntries(entries) {
    if (!Array.isArray(entries)) return [];
    return entries
        .map((entry) => ({
            site: text(entry?.site).trim(),
            download: entry?.download ?? null,
            upload: entry?.upload ?? null,
        }))
        .filter((entry) => entry.site.length > 0);
}

function buildSites() {
    const bySite = new Map();

    store.statusRows.forEach((row) => {
        bySite.set(row.site, {
            site: row.site,
            currentDownMbps: row.currentDownMbps,
            currentUpMbps: row.currentUpMbps,
            download: null,
            upload: null,
        });
    });

    store.debugEntries.forEach((entry) => {
        const existing = bySite.get(entry.site) || {
            site: entry.site,
            currentDownMbps: null,
            currentUpMbps: null,
            download: null,
            upload: null,
        };
        existing.download = entry.download;
        existing.upload = entry.upload;
        if (existing.currentDownMbps === null) {
            existing.currentDownMbps = num(entry.download?.queue_mbps);
        }
        if (existing.currentUpMbps === null) {
            existing.currentUpMbps = num(entry.upload?.queue_mbps);
        }
        bySite.set(entry.site, existing);
    });

    return Array.from(bySite.values())
        .map((site) => {
            const downloadSummary = directionSummary(site.download);
            const uploadSummary = directionSummary(site.upload);
            const ages = [num(site.download?.last_action_age_secs), num(site.upload?.last_action_age_secs)]
                .filter((value) => value !== null);
            const lastActionAgeSeconds = ages.length > 0 ? Math.min(...ages) : null;
            const actions = [text(site.download?.last_action).trim(), text(site.upload?.last_action).trim()].filter(Boolean);
            const cooldowns = [num(site.download?.cooldown_remaining_secs), num(site.upload?.cooldown_remaining_secs)]
                .filter((value) => value !== null && value > 0);
            const delayValues = [num(site.download?.delay_ms), num(site.upload?.delay_ms)]
                .filter((value) => value !== null);
            const maxDelayMs = delayValues.length > 0 ? Math.max(...delayValues) : null;
            const recentlyChanged = changedRecently(site.download) || changedRecently(site.upload);
            const inCooldown = cooldowns.length > 0;
            const siteVm = {
                ...site,
                downloadSummary,
                uploadSummary,
                lastActionAgeSeconds,
                lastActionLabel: actions[0] || "",
                changedRecently: recentlyChanged,
                inCooldown,
                maxDelayMs,
                attentionScore: 0,
            };
            siteVm.attentionScore = computeAttentionScore(siteVm);
            return siteVm;
        })
        .sort((a, b) => {
            if (b.attentionScore !== a.attentionScore) return b.attentionScore - a.attentionScore;
            return a.site.localeCompare(b.site);
        });
}

function ensureSelectedSite(sites) {
    if (sites.length === 0) {
        store.selectedSite = null;
        return null;
    }
    const existing = sites.find((site) => site.site === store.selectedSite);
    if (existing) return existing.site;
    store.selectedSite = sites[0].site;
    return store.selectedSite;
}

function buildSnapshot() {
    const sites = buildSites();
    const selectedSite = ensureSelectedSite(sites);
    const recentChanges = sites.filter((site) => site.changedRecently).length;
    const cooldownSites = sites.filter((site) => site.inCooldown).length;
    return {
        config: store.config,
        configError: store.configError,
        sites,
        selectedSite,
        singleSite: sites.length === 1,
        empty: sites.length === 0,
        watchedSiteCount: sites.length,
        cooldownSiteCount: cooldownSites,
        recentChangeCount: recentChanges,
    };
}

function notify() {
    const snapshot = buildSnapshot();
    listeners.forEach((listener) => listener(snapshot));
}

export function subscribeStormguardState(listener) {
    listeners.add(listener);
    listener(buildSnapshot());
    return () => listeners.delete(listener);
}

export function requestStormguardConfig() {
    if (store.configRequested) return;
    store.configRequested = true;
    loadConfig(
        (msg) => {
            store.config = msg?.data?.stormguard ?? window.config?.stormguard ?? null;
            notify();
        },
        () => {
            store.configError = true;
            notify();
        },
    );
}

export function updateStormguardStatus(rows) {
    if (rows === store.lastStatusPayload) return;
    store.lastStatusPayload = rows;
    store.statusRows = normalizeStatusRows(rows);
    notify();
}

export function updateStormguardDebug(entries) {
    if (entries === store.lastDebugPayload) return;
    store.lastDebugPayload = entries;
    store.debugEntries = normalizeDebugEntries(entries);
    notify();
}

export function selectStormguardSite(siteName) {
    const site = text(siteName).trim();
    store.selectedSite = site || null;
    notify();
}

export function stormguardActivityRows(snapshot, limit = 12) {
    const rows = [];
    (snapshot?.sites ?? []).forEach((site) => {
        [
            ["download", site.download, site.downloadSummary],
            ["upload", site.upload, site.uploadSummary],
        ].forEach(([directionName, direction, summary]) => {
            if (!direction) return;
            const age = num(direction.last_action_age_secs);
            const cooldown = num(direction.cooldown_remaining_secs);
            const sortAge = age !== null ? age : (cooldown !== null ? cooldown : 1e9);
            rows.push({
                site: site.site,
                direction: directionName,
                summary,
                action: text(direction.last_action).trim() || summary.label,
                ageSeconds: age,
                cooldownSeconds: cooldown,
                reason: directionReason(direction),
                sortAge,
            });
        });
    });

    return rows
        .sort((a, b) => a.sortAge - b.sortAge || a.site.localeCompare(b.site))
        .slice(0, limit);
}

export function stormguardSelectedSite(snapshot) {
    if (!snapshot?.selectedSite) return null;
    return (snapshot.sites ?? []).find((site) => site.site === snapshot.selectedSite) || null;
}

export function stormguardConfigBadgeData(config) {
    if (!config) {
        return {
            enabledLabel: "Unknown",
            enabledClass: "bg-light text-secondary border",
            strategy: "—",
            dryRunLabel: "—",
        };
    }

    return {
        enabledLabel: config.enabled ? "Enabled" : "Disabled",
        enabledClass: config.enabled
            ? "bg-success-subtle text-success border border-success-subtle"
            : "bg-light text-secondary border",
        strategy: titleCase(config.strategy || "unknown"),
        dryRunLabel: config.dry_run ? "Dry Run" : "Live",
    };
}
