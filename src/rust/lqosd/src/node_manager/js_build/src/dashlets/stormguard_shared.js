import {loadConfig} from "../config/config_helper";
import {formatNumber} from "../i18n/format.js";
import {t} from "../i18n/index.js";

const CHANGE_RECENT_WINDOW_SECONDS = 300;

// The two precisions the StormGuard sentences print: one decimal for a cooldown
// in seconds, two for a delay in milliseconds. `formatNumber` keeps exactly
// those digits and takes the decimal mark from the active locale, so a Spanish
// sentence reads "12,5 s" while the English one reads "12.5 s".
const ONE_DECIMAL = {minimumFractionDigits: 1, maximumFractionDigits: 1};
const TWO_DECIMALS = {minimumFractionDigits: 2, maximumFractionDigits: 2};
// A queue rate in the site tables: two fraction digits at most, no trailing
// zeros, so "1000" stays "1000" instead of "1000.00".
const STORMGUARD_MBPS = {maximumFractionDigits: 2};
// `Intl` percent style takes the 0–1 ratio and places the mark itself: "2.50%"
// in English, "2,50 %" in Spanish.
const STORMGUARD_PERCENT = {style: "percent", minimumFractionDigits: 2, maximumFractionDigits: 2};

// Attention weights keyed by the summary's stable `code`. The label is display
// text and changes with the locale, so nothing may compare on it.
const ATTENTION_SCORES = new Map([
    ["congestion", 60],
    ["reducing", 50],
    ["recovering", 40],
    ["at_floor", 20],
    ["at_ceiling", 10],
]);

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
    return formatNumber(n, STORMGUARD_MBPS);
}

export function formatStormguardPercent(value) {
    const n = num(value);
    if (n === null) return "—";
    return formatNumber(n, STORMGUARD_PERCENT);
}

export function formatStormguardMs(value) {
    const n = num(value);
    if (n === null) return "—";
    return `${formatNumber(n, TWO_DECIMALS)} ms`;
}

// translatable: false — "m s" and "h m" are the unit pairs the age formatter prints beside its digits, not labels
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
            code: "no_data",
            label: t("dashlet.stormguard.state.no_data"),
            className: "bg-light text-secondary border",
            reason: t("dashlet.stormguard.reason.no_data"),
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
            code: "cooling",
            label: t("dashlet.stormguard.state.cooling"),
            className: "bg-warning-subtle text-warning border border-warning-subtle",
            reason: t("tree.stormguard.cooldown_remaining", {seconds: formatNumber(cooldown, ONE_DECIMAL)}),
        };
    }

    if (action.includes("decrease")) {
        return {
            code: "reducing",
            label: t("dashlet.stormguard.state.reducing"),
            className: "bg-danger-subtle text-danger border border-danger-subtle",
            reason: t("dashlet.stormguard.reason.reduced"),
        };
    }

    if (action.includes("increase")) {
        return {
            code: "recovering",
            label: t("dashlet.stormguard.state.recovering"),
            className: "bg-success-subtle text-success border border-success-subtle",
            reason: t("dashlet.stormguard.reason.recovered"),
        };
    }

    if (queue !== null && min !== null && queue <= min && direction.can_decrease === false) {
        return {
            code: "at_floor",
            label: t("dashlet.stormguard.state.at_floor"),
            className: "bg-secondary-subtle text-secondary border border-secondary-subtle",
            reason: t("dashlet.stormguard.reason.at_floor"),
        };
    }

    if (queue !== null && max !== null && queue >= max && direction.can_increase === false) {
        return {
            code: "at_ceiling",
            label: t("dashlet.stormguard.state.at_ceiling"),
            className: "bg-primary-subtle text-primary border border-primary-subtle",
            reason: t("dashlet.stormguard.reason.at_ceiling"),
        };
    }

    if (delay !== null && delay > 5) {
        return {
            code: "congestion",
            label: t("dashlet.stormguard.state.congestion"),
            className: "bg-danger-subtle text-danger border border-danger-subtle",
            reason: t("dashlet.stormguard.reason.congestion", {ms: formatNumber(delay, TWO_DECIMALS)}),
        };
    }

    if (retrans !== null && retrans >= 0.01) {
        return {
            code: "loss",
            label: t("dashlet.stormguard.state.loss"),
            className: "bg-warning-subtle text-warning border border-warning-subtle",
            reason: t("dashlet.stormguard.reason.loss", {percent: formatStormguardPercent(retrans)}),
        };
    }

    if (state === "warmup") {
        return {
            code: "warmup",
            label: t("dashlet.stormguard.state.warmup"),
            className: "bg-info-subtle text-info border border-info-subtle",
            reason: t("dashlet.stormguard.reason.warmup"),
        };
    }

    return {
        code: "holding",
        label: t("dashlet.stormguard.state.holding"),
        className: "bg-light text-secondary border",
        reason: t("dashlet.stormguard.reason.holding"),
    };
}

export function directionReason(direction) {
    if (!direction) return t("dashlet.stormguard.detail.no_metrics");
    const cooldown = num(direction.cooldown_remaining_secs) ?? 0;
    if (cooldown > 0.05) {
        return t("dashlet.stormguard.detail.cooldown", {seconds: formatNumber(cooldown, ONE_DECIMAL)});
    }

    const delay = num(direction.delay_ms);
    if (delay !== null && delay > 0.5) {
        return t("dashlet.stormguard.detail.delay", {ms: formatNumber(delay, TWO_DECIMALS)});
    }

    const retrans = num(direction.retrans_ma) ?? num(direction.retrans);
    if (retrans !== null && retrans >= 0.005) {
        return t("dashlet.stormguard.detail.retrans", {percent: formatStormguardPercent(retrans)});
    }

    const queue = num(direction.queue_mbps);
    const min = num(direction.min_mbps);
    const max = num(direction.max_mbps);
    if (queue !== null && min !== null && queue <= min && direction.can_decrease === false) {
        return t("dashlet.stormguard.detail.at_min");
    }
    if (queue !== null && max !== null && queue >= max && direction.can_increase === false) {
        return t("dashlet.stormguard.detail.at_max");
    }

    const throughput = num(direction.throughput_ma_mbps) ?? num(direction.throughput_mbps);
    if (throughput !== null && max !== null && throughput < max * 0.2) {
        return t("dashlet.stormguard.detail.low_load");
    }

    return t("dashlet.stormguard.detail.monitoring");
}

function computeAttentionScore(site) {
    let score = 0;
    if (site.changedRecently) score += 100;
    if (site.inCooldown) score += 80;
    const codes = new Set([site.downloadSummary?.code, site.uploadSummary?.code]);
    for (const [code, weight] of ATTENTION_SCORES) {
        if (codes.has(code)) score += weight;
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
            enabledLabel: t("topology.status.unknown"),
            enabledClass: "bg-light text-secondary border",
            strategy: "—",
            dryRunLabel: "—",
        };
    }

    return {
        enabledLabel: config.enabled ? t("topology.status.enabled") : t("topology.status.disabled"),
        enabledClass: config.enabled
            ? "bg-success-subtle text-success border border-success-subtle"
            : "bg-light text-secondary border",
        strategy: titleCase(config.strategy || "unknown"),
        dryRunLabel: config.dry_run ? t("dashlet.stormguard.badge.dry_run") : t("dashlet.stormguard.badge.live"),
    };
}
