import {t} from "../i18n/index.js";
import {formatDateTime} from "../i18n/format.js";

// translatable: false — "Idle" is the bakery mode token the switch compares against; the badge renders the catalog label

export function formatUnixSecondsToLocalDateTime(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "—";
    }
    return formatDateTime(n * 1000, {
        year: "numeric",
        month: "short",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
    });
}

export function formatDurationMs(durationMs) {
    const n = typeof durationMs === "number" ? durationMs : parseInt(durationMs, 10);
    if (!Number.isFinite(n) || n < 0) {
        return "—";
    }
    if (n < 1000) {
        return `${n} ms`;
    }
    if (n < 60_000) {
        return `${(n / 1000).toFixed(1)} s`;
    }
    return `${(n / 60_000).toFixed(1)} min`;
}

export function formatElapsedSince(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "—";
    }
    const delta = Math.max(0, Math.floor(Date.now() / 1000) - n);
    if (delta < 60) {
        return t("bakery.elapsed.seconds", {seconds: delta});
    }
    if (delta < 3600) {
        return t("bakery.elapsed.min_sec", {minutes: Math.floor(delta / 60), seconds: delta % 60});
    }
    if (delta < 86_400) {
        const hours = Math.floor(delta / 3600);
        const mins = Math.floor((delta % 3600) / 60);
        return t("bakery.elapsed.hour_min", {hours, minutes: mins});
    }
    const days = Math.floor(delta / 86_400);
    const hours = Math.floor((delta % 86_400) / 3600);
    return t("bakery.elapsed.day_hour", {days, hours});
}

export function mkBadge(text, className, title = "") {
    const span = document.createElement("span");
    span.className = `badge ${className}`;
    span.textContent = text;
    if (title) {
        span.title = title;
    }
    return span;
}

export function bakeryModeBadge(modeRaw) {
    const mode = (modeRaw ?? "").toString();
    switch (mode) {
        case "ApplyingFullReload":
            return mkBadge(t("bakery.mode.full_reload"), "bg-warning-subtle text-warning border border-warning-subtle");
        case "ApplyingLiveChange":
            return mkBadge(t("bakery.mode.live_change"), "bg-info-subtle text-info border border-info-subtle");
        case "Idle":
        default:
            return mkBadge(t("scheduler.badge.idle"), "bg-light text-secondary border");
    }
}

export function bakeryPreflightBadge(preflight) {
    if (!preflight) {
        return mkBadge(t("circuit.queue_tree.unknown_node"), "bg-light text-secondary border");
    }
    if (preflight.ok) {
        return mkBadge(t("bakery.preflight.within_budget"), "bg-success-subtle text-success border border-success-subtle", preflight.message || "");
    }
    const interfaces = Array.isArray(preflight.interfaces) ? preflight.interfaces : [];
    const safeBudget = Number.isFinite(preflight.safeBudget) ? preflight.safeBudget : null;
    const overBudget = safeBudget !== null
        && interfaces.some((entry) => Number.isFinite(entry?.plannedQdiscs) && entry.plannedQdiscs > safeBudget);
    if (!overBudget && preflight.memoryOk === false) {
        return mkBadge(t("bakery.preflight.memory_blocked"), "bg-danger-subtle text-danger border border-danger-subtle", preflight.message || "");
    }
    return mkBadge(t("bakery.preflight.over_budget"), "bg-danger-subtle text-danger border border-danger-subtle", preflight.message || "");
}
