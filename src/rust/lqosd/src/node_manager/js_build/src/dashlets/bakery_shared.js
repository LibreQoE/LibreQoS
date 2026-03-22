export function formatUnixSecondsToLocalDateTime(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "—";
    }
    return new Date(n * 1000).toLocaleString(undefined, {
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
        return `${delta}s`;
    }
    if (delta < 3600) {
        return `${Math.floor(delta / 60)}m ${delta % 60}s`;
    }
    const hours = Math.floor(delta / 3600);
    const mins = Math.floor((delta % 3600) / 60);
    return `${hours}h ${mins}m`;
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
            return mkBadge("Applying Full Reload", "bg-warning-subtle text-warning border border-warning-subtle");
        case "ApplyingLiveChange":
            return mkBadge("Applying Live Change", "bg-info-subtle text-info border border-info-subtle");
        case "Idle":
        default:
            return mkBadge("Idle", "bg-light text-secondary border");
    }
}

export function bakeryPreflightBadge(preflight) {
    if (!preflight) {
        return mkBadge("Unknown", "bg-light text-secondary border");
    }
    if (preflight.ok) {
        return mkBadge("Within Budget", "bg-success-subtle text-success border border-success-subtle", preflight.message || "");
    }
    return mkBadge("Over Budget", "bg-danger-subtle text-danger border border-danger-subtle", preflight.message || "");
}
