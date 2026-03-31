import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {formatUnixSecondsToLocalDateTime, mkBadge} from "./bakery_shared";
import {renderOperationCards} from "./operation_cards";

const BAKERY_ACTIVITY_PAGE_SIZE = 10;
const BAKERY_ACTIVITY_SUMMARY_MAX_CHARS = 220;
const BAKERY_OPERATION_SUMMARY_COUNT = 6;
const BAKERY_OPERATION_MERGE_WINDOW_SECONDS = 180;
const BAKERY_ACTIVITY_VIEW_STORAGE_KEY = "lqos_bakery_activity_view";
const FULL_RELOAD_SUPPORT_EVENTS = new Set([
    "baseline_rebuild_startup",
    "baseline_rebuild_required",
    "commit_received",
]);
const LIVE_CHANGE_PREFIXES = [
    "updating circuit speeds live",
    "migrating circuits between parent nodes (fallback)",
    "treeguard runtime top-level site reparent",
    "treeguard runtime top-level circuit reparent",
    "treeguard runtime child-site shadow create",
    "treeguard runtime circuit reparent",
    "treeguard runtime hidden site restore",
    "treeguard runtime site restore",
    "treeguard runtime circuit restore",
];

function truncateSummary(summary) {
    const normalized = (summary ?? "").toString().trim();
    if (normalized.length <= BAKERY_ACTIVITY_SUMMARY_MAX_CHARS) {
        return normalized;
    }
    return normalized.slice(0, BAKERY_ACTIVITY_SUMMARY_MAX_CHARS - 3).trimEnd() + "...";
}

function summaryPrefix(summary) {
    return ((summary ?? "").toString().split(":")[0] || "").trim();
}

function isKnownLiveChangeSummary(summary) {
    const prefix = summaryPrefix(summary).toLowerCase();
    return LIVE_CHANGE_PREFIXES.some((candidate) => prefix === candidate);
}

function classifyEvent(entry) {
    const event = (entry?.event ?? "").toString().trim().toLowerCase();
    const status = (entry?.status ?? "").toString().trim().toLowerCase();
    const summary = (entry?.summary ?? "").toString().trim();
    const summaryLower = summary.toLowerCase();

    let stage = "Pipeline";
    let scope = "Bakery";
    let outcome = "Info";
    let outcomeClass = "bg-light text-secondary border";

    if (event.startsWith("preflight_")) {
        stage = "Preflight";
        scope = "Safety";
    } else if (event.includes("prune")) {
        stage = "Cleanup";
        scope = "TreeGuard";
    } else if (event.includes("started")) {
        stage = "Apply";
    } else if (event.includes("finished") || event.includes("failed")) {
        stage = "Verify";
    }

    if (event === "full_reload_started" || summaryLower.includes("full reload")) {
        scope = "Full Reload";
    } else if (
        event === "live_change_started"
        || (!summaryLower.includes("processing batch") && summaryLower.includes("live"))
        || isKnownLiveChangeSummary(summary)
    ) {
        scope = "Live Change";
    } else if (event.startsWith("runtime_site_prune_")) {
        scope = "TreeGuard Cleanup";
    } else if (event.startsWith("preflight_")) {
        scope = "Preflight";
    } else if (event.startsWith("stormguard_")) {
        scope = "StormGuard";
    } else if (summaryLower.startsWith("processing batch")) {
        scope = "Full Reload";
    }

    if (event.endsWith("_started")) {
        outcome = "Running";
        outcomeClass = "bg-primary-subtle text-primary border border-primary-subtle";
    } else if (event === "reload_required") {
        stage = "Verify";
        scope = "Full Reload";
        outcome = "Reload Required";
        outcomeClass = "bg-danger-subtle text-danger border border-danger-subtle";
    } else if (event === "reload_required_cleared") {
        stage = "Verify";
        scope = "Full Reload";
        outcome = "Cleared";
        outcomeClass = "bg-success-subtle text-success border border-success-subtle";
    } else if (event === "preflight_ok") {
        outcome = "Passed";
        outcomeClass = "bg-success-subtle text-success border border-success-subtle";
    } else if (event === "preflight_blocked") {
        outcome = "Blocked";
        outcomeClass = "bg-warning-subtle text-warning border border-warning-subtle";
    } else if (event === "runtime_site_prune_retry") {
        outcome = "Retrying";
        outcomeClass = "bg-warning-subtle text-warning border border-warning-subtle";
    } else if (event === "runtime_site_prune_dirty") {
        outcome = "Dirty";
        outcomeClass = "bg-danger-subtle text-danger border border-danger-subtle";
    } else if (event === "runtime_site_prune_completed" || event === "apply_finished") {
        outcome = "Completed";
        outcomeClass = "bg-success-subtle text-success border border-success-subtle";
    } else if (event === "apply_failed" || status === "error") {
        outcome = "Failed";
        outcomeClass = "bg-danger-subtle text-danger border border-danger-subtle";
    } else if (status === "warning") {
        outcome = "Warning";
        outcomeClass = "bg-warning-subtle text-warning border border-warning-subtle";
    }

    return {
        stage,
        scope,
        outcome,
        outcomeClass,
        event,
    };
}

function humanizeLabel(raw, fallback = "Bakery update") {
    const normalized = (raw ?? "").toString().trim();
    if (!normalized) {
        return fallback;
    }

    const spaced = normalized
        .replace(/[|:_-]+/g, " ")
        .replace(/\s+/g, " ")
        .trim();

    return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

function extractSiteToken(summary) {
    const match = (summary ?? "").toString().match(/\bsite\s+(-?\d+)\b/i);
    return match ? match[1] : null;
}

function resolvedSiteName(entry) {
    return (entry?.siteName ?? "").toString().trim();
}

function displaySummary(entry) {
    const summary = (entry?.summary ?? "—").toString();
    const siteName = resolvedSiteName(entry);
    if (!siteName) {
        return summary;
    }
    return summary.replace(/\bsite\s+-?\d+\b/i, `site ${siteName}`);
}

function describeOperation(entry, meta) {
    const summary = (entry?.summary ?? "").toString().trim();
    const summaryLower = summary.toLowerCase();
    const prefix = summaryPrefix(summary);
    const siteToken = extractSiteToken(summary);
    const siteName = resolvedSiteName(entry);
    const event = meta.event;

    if (event.startsWith("preflight_")) {
        return {
            kind: "preflight",
            key: "preflight",
            label: "Reload safety check",
            scopeLabel: "Preflight",
        };
    }

    if (
        event === "full_reload_trigger"
        || event === "full_reload_started"
        || event === "reload_required"
        || event === "reload_required_cleared"
        || meta.scope === "Full Reload"
    ) {
        return {
            kind: "full_reload",
            key: "full_reload",
            label: "Full reload",
            scopeLabel: "Full Reload",
        };
    }

    if (event.startsWith("runtime_site_prune_")) {
        const label = siteName ? `Cleanup ${siteName}` : (siteToken ? `Cleanup site ${siteToken}` : "Deferred cleanup");
        return {
            kind: "runtime_cleanup",
            key: `runtime_cleanup:${siteName || siteToken || label}`,
            label,
            scopeLabel: "Cleanup",
        };
    }

    if (summaryLower.includes("treeguard runtime hidden site restore")) {
        const label = siteName ? `Restore ${siteName}` : (siteToken ? `Restore site ${siteToken}` : "Restore hidden site");
        return {
            kind: "runtime_restore",
            key: `runtime_restore:${siteName || siteToken || prefix}`,
            label,
            scopeLabel: "Live Change",
        };
    }

    if (meta.scope === "Live Change" || summaryLower.includes("treeguard runtime")) {
        const label = humanizeLabel(prefix || "Live topology change", "Live topology change");
        return {
            kind: "live_change",
            key: `live_change:${siteToken ?? label}`,
            label,
            scopeLabel: "Live Change",
        };
    }

    if (meta.scope === "StormGuard") {
        return {
            kind: "stormguard",
            key: `stormguard:${prefix || event}`,
            label: humanizeLabel(prefix || "StormGuard update", "StormGuard update"),
            scopeLabel: "StormGuard",
        };
    }

    return {
        kind: "bakery",
        key: `bakery:${prefix || event || meta.scope}`,
        label: humanizeLabel(prefix || meta.scope || "Bakery update", "Bakery update"),
        scopeLabel: meta.scope,
    };
}

function shouldFoldIntoFullReload(entry, descriptor, meta) {
    if (descriptor.kind === "preflight") {
        return meta.outcome !== "Blocked";
    }

    return descriptor.kind === "bakery" && FULL_RELOAD_SUPPORT_EVENTS.has(meta.event);
}

function findMergeTarget(groups, entry, descriptor, meta) {
    const entryTs = entry?.ts ?? 0;

    if (shouldFoldIntoFullReload(entry, descriptor, meta)) {
        return groups.find((group) =>
            group.kind === "full_reload"
            && Math.abs((group.oldestTs ?? 0) - entryTs) <= BAKERY_OPERATION_MERGE_WINDOW_SECONDS);
    }

    return groups.find((group) =>
        group.key === descriptor.key
        && Math.abs((group.oldestTs ?? 0) - entryTs) <= BAKERY_OPERATION_MERGE_WINDOW_SECONDS);
}

function operationStageLabels(group) {
    switch (group.kind) {
        case "preflight":
            return ["Queued", "Checking", "Done"];
        default:
            return ["Queued", "Applying", "Verifying", "Cleanup", "Done"];
    }
}

function operationShowsProgress(group) {
    return group.kind === "full_reload"
        || group.kind === "preflight"
        || group.kind === "live_change"
        || group.kind === "runtime_cleanup"
        || group.kind === "runtime_restore";
}

function operationProgressStep(group) {
    const outcome = group.latestMeta?.outcome;
    const stage = group.latestMeta?.stage;
    if (group.kind === "preflight") {
        if (outcome === "Passed" || outcome === "Blocked") {
            return 2;
        }
        return 1;
    }

    if (outcome === "Completed" || outcome === "Cleared") {
        return 4;
    }
    if (stage === "Cleanup") {
        return 3;
    }
    if (stage === "Verify") {
        return 2;
    }
    if (stage === "Apply") {
        return 1;
    }
    return 0;
}

function operationProgressPercent(group) {
    const labels = operationStageLabels(group);
    const divisor = Math.max(1, labels.length - 1);
    return (operationProgressStep(group) / divisor) * 100;
}

function operationProgressBarClass(group) {
    const outcome = group.latestMeta?.outcome;
    switch (outcome) {
        case "Completed":
        case "Passed":
        case "Cleared":
            return "bg-success";
        case "Failed":
        case "Dirty":
        case "Reload Required":
            return "bg-danger";
        case "Blocked":
        case "Retrying":
        case "Warning":
            return "bg-warning";
        default:
            return "bg-info";
    }
}

function buildOperationSummaries(entries) {
    const groups = [];

    entries.forEach((entry) => {
        const meta = classifyEvent(entry);
        const descriptor = describeOperation(entry, meta);
        const targetGroup = findMergeTarget(groups, entry, descriptor, meta);

        if (targetGroup) {
            targetGroup.events.push(entry);
            targetGroup.oldestTs = Math.min(targetGroup.oldestTs, entry?.ts ?? targetGroup.oldestTs);
            return;
        }

        groups.push({
            kind: descriptor.kind,
            key: descriptor.key,
            label: descriptor.label,
            scopeLabel: descriptor.scopeLabel,
            latestEntry: entry,
            latestMeta: meta,
            oldestTs: entry?.ts ?? 0,
            events: [entry],
        });
    });

    return groups.slice(0, BAKERY_OPERATION_SUMMARY_COUNT);
}

function loadViewMode() {
    try {
        const saved = window?.localStorage?.getItem(BAKERY_ACTIVITY_VIEW_STORAGE_KEY);
        return saved === "events" ? "events" : "operations";
    } catch (_) {
        return "operations";
    }
}

export class BakeryActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.entries = [];
        this.currentPage = 0;
        this.viewMode = loadViewMode();
    }

    title() {
        return "Recent Operations";
    }

    tooltip() {
        return "<h5>Recent Operations</h5><p>Summarizes recent Bakery work as human-readable operations with simple progress, while keeping the detailed event log available underneath for operators who need the raw feed.</p>";
    }

    subscribeTo() {
        return ["BakeryActivity"];
    }

    buildContainer() {
        const base = super.buildContainer();
        base.classList.add("dashbox-body-scroll");

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const viewControls = document.createElement("div");
        viewControls.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "flex-wrap", "mb-3");

        const operationsHeader = document.createElement("div");
        operationsHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        operationsHeader.textContent = "View";
        operationsHeader.classList.remove("mb-2");
        operationsHeader.classList.add("mb-0");
        viewControls.appendChild(operationsHeader);

        const toggleGroup = document.createElement("div");
        toggleGroup.classList.add("btn-group", "btn-group-sm");

        this.operationsButton = document.createElement("button");
        this.operationsButton.type = "button";
        this.operationsButton.textContent = "Operations";
        this.operationsButton.addEventListener("click", () => this.setViewMode("operations"));

        this.eventsButton = document.createElement("button");
        this.eventsButton.type = "button";
        this.eventsButton.textContent = "Event Log";
        this.eventsButton.addEventListener("click", () => this.setViewMode("events"));

        toggleGroup.appendChild(this.operationsButton);
        toggleGroup.appendChild(this.eventsButton);
        viewControls.appendChild(toggleGroup);
        wrap.appendChild(viewControls);

        this.operationsSection = document.createElement("div");
        const operationsSectionHeader = document.createElement("div");
        operationsSectionHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        operationsSectionHeader.textContent = "Recent Operations";
        this.operationsSection.appendChild(operationsSectionHeader);

        this.operationsList = document.createElement("div");
        this.operationsList.classList.add("d-flex", "flex-column", "gap-2", "mb-3");
        this.operationsSection.appendChild(this.operationsList);
        wrap.appendChild(this.operationsSection);

        this.eventsSection = document.createElement("div");
        const detailHeader = document.createElement("div");
        detailHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        detailHeader.textContent = "Detailed Events";
        this.eventsSection.appendChild(detailHeader);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        const hr = document.createElement("tr");
        ["Local Time", "Stage", "Outcome", "Scope", "Summary"].forEach((label) => {
            const th = document.createElement("th");
            th.textContent = label;
            hr.appendChild(th);
        });
        thead.appendChild(hr);

        this.tbody = document.createElement("tbody");
        table.appendChild(thead);
        table.appendChild(this.tbody);
        tableWrap.appendChild(table);
        this.eventsSection.appendChild(tableWrap);

        const footer = document.createElement("div");
        footer.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "pt-2", "flex-wrap");

        this.pageSummary = document.createElement("div");
        this.pageSummary.classList.add("small", "text-secondary");
        footer.appendChild(this.pageSummary);

        const pager = document.createElement("div");
        pager.classList.add("btn-group", "btn-group-sm");

        this.prevButton = document.createElement("button");
        this.prevButton.type = "button";
        this.prevButton.classList.add("btn", "btn-outline-secondary");
        this.prevButton.innerHTML = "<i class='fa fa-chevron-left'></i> Newer";
        this.prevButton.setAttribute("aria-label", "Show newer Bakery events");
        this.prevButton.addEventListener("click", () => {
            if (this.currentPage > 0) {
                this.currentPage -= 1;
                this.renderPage();
            }
        });

        this.nextButton = document.createElement("button");
        this.nextButton.type = "button";
        this.nextButton.classList.add("btn", "btn-outline-secondary");
        this.nextButton.innerHTML = "Older <i class='fa fa-chevron-right'></i>";
        this.nextButton.setAttribute("aria-label", "Show older Bakery events");
        this.nextButton.addEventListener("click", () => {
            if (this.currentPage + 1 < this.totalPages()) {
                this.currentPage += 1;
                this.renderPage();
            }
        });

        pager.appendChild(this.prevButton);
        pager.appendChild(this.nextButton);
        footer.appendChild(pager);
        this.eventsSection.appendChild(footer);
        wrap.appendChild(this.eventsSection);
        base.appendChild(wrap);
        this.renderViewMode();
        return base;
    }

    setViewMode(mode) {
        this.viewMode = mode === "events" ? "events" : "operations";
        try {
            window?.localStorage?.setItem(BAKERY_ACTIVITY_VIEW_STORAGE_KEY, this.viewMode);
        } catch (_) {}
        this.renderViewMode();
    }

    renderViewMode() {
        if (this.operationsSection) {
            this.operationsSection.classList.toggle("d-none", this.viewMode !== "operations");
        }
        if (this.eventsSection) {
            this.eventsSection.classList.toggle("d-none", this.viewMode !== "events");
        }
        if (this.operationsButton) {
            this.operationsButton.className = this.viewMode === "operations" ? "btn btn-primary" : "btn btn-outline-secondary";
        }
        if (this.eventsButton) {
            this.eventsButton.className = this.viewMode === "events" ? "btn btn-primary" : "btn btn-outline-secondary";
        }
    }

    totalPages() {
        return Math.max(1, Math.ceil(this.entries.length / BAKERY_ACTIVITY_PAGE_SIZE));
    }

    renderPage() {
        const entries = this.entries;
        const totalRows = entries.length;
        const totalPages = this.totalPages();
        this.currentPage = Math.min(this.currentPage, totalPages - 1);

        this.tbody.innerHTML = "";
        if (totalRows === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 5;
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            this.pageSummary.textContent = "0 events";
            this.prevButton.disabled = true;
            this.nextButton.disabled = true;
            this.renderOperationSummaries([]);
            this.renderViewMode();
            return;
        }

        const start = this.currentPage * BAKERY_ACTIVITY_PAGE_SIZE;
        const end = Math.min(start + BAKERY_ACTIVITY_PAGE_SIZE, totalRows);
        const visibleEntries = entries.slice(start, end);

        visibleEntries.forEach((entry) => {
            const meta = classifyEvent(entry);
            const tr = document.createElement("tr");
            const tdTime = document.createElement("td");
            const tdStage = document.createElement("td");
            const tdOutcome = document.createElement("td");
            const tdScope = document.createElement("td");
            const tdSummary = document.createElement("td");

            tdTime.textContent = formatUnixSecondsToLocalDateTime(entry?.ts);
            tdStage.appendChild(
                mkBadge(meta.stage, "bg-body-tertiary text-body-secondary border", meta.event || ""),
            );
            tdOutcome.appendChild(
                mkBadge(meta.outcome, meta.outcomeClass, entry?.summary || meta.event || ""),
            );
            tdScope.appendChild(
                mkBadge(meta.scope, "bg-info-subtle text-info border border-info-subtle", entry?.event || ""),
            );
            const fullSummary = displaySummary(entry);
            tdSummary.textContent = truncateSummary(fullSummary);
            tdSummary.title = fullSummary;

            tr.appendChild(tdTime);
            tr.appendChild(tdStage);
            tr.appendChild(tdOutcome);
            tr.appendChild(tdScope);
            tr.appendChild(tdSummary);
            this.tbody.appendChild(tr);
        });

        this.pageSummary.textContent = `${start + 1}-${end} of ${totalRows} events`;
        this.prevButton.disabled = this.currentPage === 0;
        this.nextButton.disabled = this.currentPage + 1 >= totalPages;
        this.renderOperationSummaries(entries);
        this.renderViewMode();
    }

    renderOperationSummaries(entries) {
        const groups = buildOperationSummaries(entries);
        const cards = groups.map((group) => {
            const eventCount = group.events.length;
            return {
                label: group.label,
                outcomeLabel: group.latestMeta?.outcome || "Info",
                outcomeClass: group.latestMeta?.outcomeClass || "bg-light text-secondary border",
                outcomeTitle: group.latestEntry?.summary || "",
                summary: truncateSummary(displaySummary(group.latestEntry)),
                summaryTitle: displaySummary(group.latestEntry),
                footerLeft: `${group.scopeLabel} • ${eventCount} event${eventCount === 1 ? "" : "s"}`,
                footerRight: formatUnixSecondsToLocalDateTime(group.latestEntry?.ts),
                stages: operationShowsProgress(group) ? operationStageLabels(group) : [],
                progressPercent: operationShowsProgress(group) ? operationProgressPercent(group) : 0,
                progressBarClass: operationShowsProgress(group) ? operationProgressBarClass(group) : "bg-info",
            };
        });
        renderOperationCards(this.operationsList, cards, { emptyText: "No recent operations" });
    }

    onMessage(msg) {
        if (msg.event !== "BakeryActivity") {
            return;
        }

        this.entries = Array.isArray(msg.data) ? msg.data : [];
        this.renderPage();
    }
}
