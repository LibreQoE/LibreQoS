import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {get_ws_client} from "../pubsub/ws";
import {mkBadge} from "./bakery_shared";
import {renderOperationCards} from "./operation_cards";

const TREEGUARD_OPERATION_MERGE_WINDOW_SECONDS = 180;
const TREEGUARD_ACTIVITY_VIEW_STORAGE_KEY = "lqos_treeguard_activity_view";

function formatUnixSecondsToLocalTime(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "";
    }
    return new Date(n * 1000).toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
    });
}

function parseCircuitEntityId(entityIdRaw) {
    const s = (entityIdRaw ?? "").toString().trim();
    if (!s) return { circuitId: "", display: "", hasName: false };

    const m = s.match(/^(.*)\s*\(([^)]+)\)\s*$/);
    if (m) {
        const name = (m[1] ?? "").toString().trim();
        const id = (m[2] ?? "").toString().trim();
        if (id) {
            return { circuitId: id, display: name || id, hasName: !!name };
        }
    }

    return { circuitId: s, display: s, hasName: false };
}

function splitOnce(s, sep) {
    const str = (s ?? "").toString();
    const idx = str.indexOf(sep);
    if (idx === -1) return [str, ""];
    return [str.slice(0, idx), str.slice(idx + sep.length)];
}

function parseDirectionalSqmToken(token) {
    const t = (token ?? "").toString().trim();
    if (!t) return { down: "", up: "" };
    if (!t.includes("/")) return { down: t, up: t };
    const [down, up] = splitOnce(t, "/");
    return { down: (down ?? "").toString().trim(), up: (up ?? "").toString().trim() };
}

function formatSqmLabel(prefix, token) {
    const { down, up } = parseDirectionalSqmToken(token);
    if (!down && !up) return prefix;
    if (down === up) return `${prefix}: ${down}`;
    return `${prefix}: DL ${down}, UL ${up}`;
}

function mkIcon(iconClass, extraClasses = []) {
    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconClass);
    extraClasses.forEach((c) => icon.classList.add(c));
    return icon;
}

function formatReason(reasonRaw) {
    const raw = (reasonRaw ?? "").toString().trim();
    if (!raw) return { label: "", title: "" };

    const lower = raw.toLowerCase();
    if (!lower.includes("next_allowed_unix=")) {
        return { label: raw, title: "" };
    }

    const m = raw.match(/next_allowed_unix=(\d+)/i);
    if (!m) return { label: raw, title: "" };

    const next = formatUnixSecondsToLocalTime(parseInt(m[1], 10));
    if (!next) return { label: raw, title: "" };

    if (lower.startsWith("reload cooldown active")) {
        return { label: `Reload on cooldown - next allowed ${next}`, title: raw };
    }

    return { label: raw.replace(m[0], `next allowed ${next}`), title: raw };
}

function loadViewMode() {
    try {
        const saved = window?.localStorage?.getItem(TREEGUARD_ACTIVITY_VIEW_STORAGE_KEY);
        return saved === "events" ? "events" : "operations";
    } catch (_) {
        return "operations";
    }
}

function classifyOutcome(entry, action) {
    const rawAction = (entry?.action ?? "").toString().trim().toLowerCase();
    const reasonRaw = (entry?.reason ?? "").toString().trim();
    const reasonLower = reasonRaw.toLowerCase();

    if (rawAction.startsWith("would_")) {
        return {
            label: "Dry Run",
            className: "bg-light text-secondary border",
            detail: null,
        };
    }

    if (rawAction.endsWith("_requested")) {
        return {
            label: "Queued",
            className: "bg-primary-subtle text-primary border border-primary-subtle",
            detail: mkBadge("Bakery", "bg-info-subtle text-info border border-info-subtle"),
        };
    }

    if (rawAction === "reload_skipped") {
        return {
            label: "Skipped",
            className: "bg-light text-secondary border",
            detail: null,
        };
    }

    if (rawAction.endsWith("_failed") || rawAction.includes("failed")) {
        return {
            label: "Failed",
            className: "bg-danger-subtle text-danger border border-danger-subtle",
            detail: null,
        };
    }

    if (reasonLower.includes("cleanup pending") || reasonLower.includes("awaiting cleanup")) {
        return {
            label: "Cleanup Pending",
            className: "bg-warning-subtle text-warning border border-warning-subtle",
            detail: mkBadge("Live", "bg-info-subtle text-info border border-info-subtle"),
        };
    }

    if (rawAction === "dry_run_toggled") {
        return {
            label: "Updated",
            className: "bg-primary-subtle text-primary border border-primary-subtle",
            detail: null,
        };
    }

    const actionLower = (action?.label ?? "").toLowerCase();
    const isLiveIntent = actionLower.includes("virtualize")
        || actionLower.includes("sqm live")
        || actionLower.includes("reload");
    const detail = isLiveIntent
        ? mkBadge("Live", "bg-info-subtle text-info border border-info-subtle")
        : (entry?.persisted
            ? mkBadge("Stored", "bg-primary-subtle text-primary border border-primary-subtle")
            : null);

    return {
        label: "Applied",
        className: "bg-success-subtle text-success border border-success-subtle",
        detail,
    };
}

function renderAction(actionRaw) {
    const raw = (actionRaw ?? "").toString();
    const [verbRaw, payloadRaw] = splitOnce(raw, ":");
    const verb = (verbRaw ?? "").toString().trim();
    const payload = (payloadRaw ?? "").toString().trim();

    const lowerVerb = verb.toLowerCase();
    let iconClass = "fa-question-circle";
    let iconExtra = ["text-muted"];
    let label = raw;

    const isFailed = lowerVerb.endsWith("_failed") || lowerVerb.includes("failed");

    if (lowerVerb === "virtualize") {
        iconClass = "fa-compress";
        iconExtra = [];
        label = "Virtualize";
    } else if (lowerVerb === "virtualize_requested") {
        iconClass = "fa-hourglass-half";
        iconExtra = ["text-primary"];
        label = "Queued virtualization";
    } else if (lowerVerb === "unvirtualize") {
        iconClass = "fa-expand";
        iconExtra = [];
        label = "Unvirtualize";
    } else if (lowerVerb === "unvirtualize_requested") {
        iconClass = "fa-hourglass-half";
        iconExtra = ["text-primary"];
        label = "Queued restore";
    } else if (lowerVerb === "dry_run_toggled") {
        iconClass = "fa-toggle-on";
        iconExtra = ["text-muted"];
        label = "Dry-run toggled";
    } else if (lowerVerb === "reload_success") {
        iconClass = "fa-refresh";
        iconExtra = ["text-success"];
        label = "Reload success";
    } else if (lowerVerb === "reload_skipped") {
        iconClass = "fa-refresh";
        iconExtra = ["text-muted"];
        label = "Reload skipped";
    } else if (lowerVerb === "reload_failed") {
        iconClass = "fa-refresh";
        iconExtra = ["text-danger"];
        label = "Reload failed";
    } else if (lowerVerb.startsWith("clear_virtual_override")) {
        iconClass = "fa-eraser";
        iconExtra = ["text-warning"];
        label = "Clear virtual override";
        if (lowerVerb.endsWith("_conflict")) label += " (conflict)";
        if (lowerVerb.endsWith("_failed")) label += " failed";
    } else if (lowerVerb.startsWith("set_virtual_override")) {
        iconClass = "fa-compress";
        iconExtra = isFailed ? ["text-danger"] : [];
        label = isFailed ? "Set virtual override failed" : "Set virtual override";
    } else if (lowerVerb.startsWith("clear_sqm_overrides")) {
        iconClass = "fa-eraser";
        iconExtra = ["text-warning"];
        label = "Clear SQM overrides";
        if (lowerVerb.endsWith("_conflict")) label += " (conflict)";
    } else if (lowerVerb === "set_sqm_override_failed") {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = "SQM override failed";
    } else if (lowerVerb === "apply_sqm_live_failed") {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = formatSqmLabel("SQM live apply failed", payload);
    } else if (lowerVerb === "would_set_sqm_override") {
        iconClass = "fa-eye";
        iconExtra = ["text-muted"];
        label = formatSqmLabel("Dry-run SQM override", payload);
    } else if (lowerVerb === "set_sqm_override") {
        if (payload.toLowerCase().includes("cake")) {
            iconClass = "fa-birthday-cake";
        } else if (payload.toLowerCase().includes("fq_codel")) {
            iconClass = "fa-tachometer";
        } else {
            iconClass = "fa-sliders";
        }
        iconExtra = [];
        label = formatSqmLabel("SQM override", payload);
    } else if (lowerVerb === "set_sqm_live") {
        iconClass = "fa-bolt";
        iconExtra = [];
        label = formatSqmLabel("SQM live apply", payload);
    } else if (isFailed) {
        iconClass = "fa-exclamation-circle";
        iconExtra = ["text-danger"];
        label = verb;
    }

    return { raw, label, iconClass, iconExtra };
}

function normalizedActionFamily(actionRaw) {
    const raw = (actionRaw ?? "").toString().trim().toLowerCase();
    const [verbRaw] = splitOnce(raw, ":");
    let verb = (verbRaw ?? "").trim();
    if (!verb) return "activity";

    if (verb.startsWith("would_")) {
        verb = verb.slice("would_".length);
    }

    if (
        verb.startsWith("set_sqm_")
        || verb.startsWith("clear_sqm_")
        || verb.startsWith("apply_sqm_")
    ) {
        return "sqm";
    }

    if (verb.startsWith("reload")) {
        return "reload";
    }

    return verb
        .replace(/_requested$/, "")
        .replace(/_failed$/, "")
        .replace(/_conflict$/, "");
}

function entityLabel(entry) {
    const entityType = (entry?.entity_type ?? "").toString().trim().toLowerCase();
    const entityId = (entry?.entity_id ?? "").toString().trim();
    if (!entityId) {
        return "";
    }
    if (entityType === "circuit") {
        return parseCircuitEntityId(entityId).display || entityId;
    }
    return entityId;
}

function isSqmBatchEntry(entry) {
    return (entry?.batchKind ?? "").toString().trim().toLowerCase() === "sqm"
        && !!(entry?.batchId ?? "").toString().trim();
}

function describeTreeGuardOperation(entry, action) {
    const family = normalizedActionFamily(entry?.action);
    const target = entityLabel(entry);
    const scopedTarget = target ? ` ${target}` : "";
    const rawAction = (entry?.action ?? "").toString().trim().toLowerCase();

    if (isSqmBatchEntry(entry)) {
        return {
            kind: "sqm_batch",
            key: `sqm_batch:${entry.batchId}`,
            label: "SQM change batch",
            scopeLabel: "Circuits",
            stages: rawAction.startsWith("would_")
                ? ["Observed", "Would Apply"]
                : ["Queued", "Applied", "Cleanup", "Done"],
        };
    }

    if (rawAction.startsWith("would_")) {
        return {
            kind: "dry_run",
            key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
            label: target ? `${action.label} ${target}` : action.label,
            scopeLabel: (entry?.entity_type ?? "TreeGuard").toString(),
            stages: ["Observed", "Would Apply"],
        };
    }

    switch (family) {
        case "virtualize":
            return {
                kind: "mutation",
                key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
                label: `Virtualize${scopedTarget}`,
                scopeLabel: (entry?.entity_type ?? "Node").toString(),
                stages: ["Queued", "Applied", "Cleanup", "Done"],
            };
        case "unvirtualize":
            return {
                kind: "mutation",
                key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
                label: `Restore${scopedTarget}`,
                scopeLabel: (entry?.entity_type ?? "Node").toString(),
                stages: ["Queued", "Applied", "Cleanup", "Done"],
            };
        case "sqm":
            return {
                kind: "sqm",
                key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
                label: `SQM change${scopedTarget}`,
                scopeLabel: (entry?.entity_type ?? "Circuit").toString(),
                stages: ["Queued", "Applied", "Cleanup", "Done"],
            };
        case "reload":
            return {
                kind: "reload",
                key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
                label: target ? `Reload ${target}` : "Reload",
                scopeLabel: "Reload",
                stages: ["Requested", "Applied", "Done"],
            };
        case "dry_run_toggled":
            return {
                kind: "config",
                key: family,
                label: "Dry-run toggled",
                scopeLabel: "Config",
                stages: [],
            };
        default:
            return {
                kind: "activity",
                key: `${family}:${(entry?.entity_type ?? "").toString()}:${(entry?.entity_id ?? "").toString()}`,
                label: target ? `${action.label} ${target}` : action.label,
                scopeLabel: (entry?.entity_type ?? "TreeGuard").toString(),
                stages: ["Queued", "Applied", "Cleanup", "Done"],
            };
    }
}

function summarizeSqmBatch(group) {
    const uniqueCircuits = new Set(
        group.events
            .map((entry) => (entry?.entity_id ?? "").toString().trim())
            .filter(Boolean),
    ).size;
    const failed = group.events.filter((entry) => {
        const action = renderAction(entry.action);
        return classifyOutcome(entry, action).label === "Failed";
    }).length;
    const dryRun = group.events.filter((entry) => {
        const action = renderAction(entry.action);
        return classifyOutcome(entry, action).label === "Dry Run";
    }).length;
    const applied = group.events.filter((entry) => {
        const action = renderAction(entry.action);
        return classifyOutcome(entry, action).label === "Applied";
    }).length;
    const queued = group.events.filter((entry) => {
        const action = renderAction(entry.action);
        return classifyOutcome(entry, action).label === "Queued";
    }).length;

    const parts = [`${uniqueCircuits} circuit${uniqueCircuits === 1 ? "" : "s"}`];
    if (applied > 0) parts.push(`${applied} applied`);
    if (failed > 0) parts.push(`${failed} failed`);
    if (dryRun > 0) parts.push(`${dryRun} dry-run`);
    if (queued > 0) parts.push(`${queued} queued`);
    return parts.join(" • ");
}

function treeguardProgressPercent(group) {
    const outcome = group.outcome.label;
    if (group.kind === "dry_run") return 100;
    if (outcome === "Queued") return 8;
    if (outcome === "Cleanup Pending") return 75;
    if (outcome === "Failed") return 50;
    if (outcome === "Dry Run") return 100;
    if (outcome === "Applied" || outcome === "Updated" || outcome === "Skipped") return 100;
    return 55;
}

function treeguardProgressClass(group) {
    switch (group.outcome.label) {
        case "Applied":
        case "Updated":
        case "Skipped":
            return "bg-success";
        case "Dry Run":
            return "bg-secondary";
        case "Failed":
            return "bg-danger";
        case "Cleanup Pending":
            return "bg-warning";
        case "Queued":
            return "bg-primary";
        default:
            return "bg-info";
    }
}

function buildTreeGuardOperationGroups(entries) {
    const groups = [];
    const recent = Array.isArray(entries) ? entries.slice(0, 50) : [];

    recent.forEach((entry) => {
        const action = renderAction(entry.action);
        const descriptor = describeTreeGuardOperation(entry, action);
        const outcome = classifyOutcome(entry, action);
        const reason = formatReason(entry.reason);
        const ts = toNumber(entry.time, 0);
        const match = groups.find((group) =>
            group.key === descriptor.key
            && Math.abs((group.oldestTs ?? 0) - ts) <= TREEGUARD_OPERATION_MERGE_WINDOW_SECONDS);

        if (match) {
            match.events.push(entry);
            match.latestEntry = entry;
            match.latestAction = action;
            match.outcome = outcome;
            match.reason = reason;
            match.oldestTs = Math.min(match.oldestTs, ts);
            return;
        }

        groups.push({
            ...descriptor,
            latestEntry: entry,
            latestAction: action,
            outcome,
            reason,
            oldestTs: ts,
            events: [entry],
        });
    });

    return groups;
}

export class TreeGuardActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.nodeIdByName = new Map();
        this.lastEntries = [];
        this.viewMode = loadViewMode();
    }

    title() {
        return "TreeGuard Activity";
    }

    tooltip() {
        return "<h5>TreeGuard Activity</h5><p>Recent TreeGuard intents with explicit outcomes so operators can distinguish queued requests, dry-runs, successful applies, cleanup-pending actions, skips, and failures.</p>";
    }

    subscribeTo() {
        return ["TreeGuardActivity"];
    }

    setup() {
        const wsClient = get_ws_client();
        const nodeWrapped = (msg) => {
            wsClient.off("NodeDirectory", nodeWrapped);
            const data = msg && Array.isArray(msg.data) ? msg.data : [];
            data.forEach((entry) => {
                const id = entry?.tree_index;
                const name = (entry?.node_name ?? "").trim();
                if (!name) return;
                if (!this.nodeIdByName.has(name)) {
                    this.nodeIdByName.set(name, id);
                }
            });
            this.rerenderWithMetadata();
        };
        wsClient.on("NodeDirectory", nodeWrapped);
        wsClient.send({ NodeDirectory: {} });
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const viewControls = document.createElement("div");
        viewControls.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "flex-wrap", "mb-3");

        const viewLabel = document.createElement("div");
        viewLabel.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-0");
        viewLabel.textContent = "View";
        viewControls.appendChild(viewLabel);

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
        const operationsHeader = document.createElement("div");
        operationsHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        operationsHeader.textContent = "Recent Operations";
        this.operationsSection.appendChild(operationsHeader);

        this.operationsList = document.createElement("div");
        this.operationsList.classList.add("d-flex", "flex-column", "gap-2");
        this.operationsSection.appendChild(this.operationsList);
        wrap.appendChild(this.operationsSection);

        this.eventsSection = document.createElement("div");
        const eventHeader = document.createElement("div");
        eventHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        eventHeader.textContent = "Detailed Events";
        this.eventsSection.appendChild(eventHeader);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        thead.classList.add("small");
        const headRow = document.createElement("tr");
        ["Local Time", "Target", "Intent", "Outcome", "Why"].forEach((header) => {
            const th = document.createElement("th");
            th.textContent = header;
            headRow.appendChild(th);
        });
        thead.appendChild(headRow);

        this.tbody = document.createElement("tbody");
        table.appendChild(thead);
        table.appendChild(this.tbody);
        tableWrap.appendChild(table);
        this.eventsSection.appendChild(tableWrap);
        wrap.appendChild(this.eventsSection);
        base.appendChild(wrap);
        this.renderViewMode();
        return base;
    }

    setViewMode(mode) {
        this.viewMode = mode === "events" ? "events" : "operations";
        try {
            window?.localStorage?.setItem(TREEGUARD_ACTIVITY_VIEW_STORAGE_KEY, this.viewMode);
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

    onMessage(msg) {
        if (msg.event !== "TreeGuardActivity") {
            return;
        }

        const entries = Array.isArray(msg.data) ? msg.data : [];
        this.lastEntries = entries;
        this.renderEntries(entries);
    }

    rerenderWithMetadata() {
        if (this.lastEntries.length > 0) {
            this.renderEntries(this.lastEntries);
        }
    }

    renderEntries(entries) {
        const groups = buildTreeGuardOperationGroups(entries).map((group) => {
            const fullReason = group.kind === "sqm_batch"
                ? summarizeSqmBatch(group)
                : (group.reason.label || "");
            const eventCount = group.events.length;
            const footerLeft = group.kind === "sqm_batch"
                ? `Circuits • ${eventCount} event${eventCount === 1 ? "" : "s"}`
                : `${group.scopeLabel} • ${eventCount} event${eventCount === 1 ? "" : "s"}`;
            return {
                label: group.label,
                outcomeLabel: group.outcome.label,
                outcomeClass: group.outcome.className,
                outcomeTitle: group.latestEntry?.reason || "",
                summary: fullReason,
                summaryTitle: group.reason.title || fullReason,
                footerLeft,
                footerRight: formatUnixSecondsToLocalTime(group.latestEntry?.time),
                stages: group.stages,
                progressPercent: group.stages.length > 0 ? treeguardProgressPercent(group) : 0,
                progressBarClass: treeguardProgressClass(group),
            };
        });
        renderOperationCards(this.operationsList, groups, { emptyText: "No recent operations" });

        this.tbody.innerHTML = "";

        if (entries.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 5;
            td.classList.add("text-muted");
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            this.renderViewMode();
            return;
        }

        entries.slice(0, 50).forEach((entry) => {
            const tr = document.createElement("tr");
            tr.classList.add("small");

            const tdTime = document.createElement("td");
            tdTime.textContent = formatUnixSecondsToLocalTime(entry.time);

            const tdEntity = document.createElement("td");
            const entityTypeRaw = (entry.entity_type ?? "").toString();
            const entityIdRaw = (entry.entity_id ?? "").toString();
            const entityType = entityTypeRaw.toLowerCase().trim();
            const entityId = entityIdRaw.trim();

            const prefix = document.createElement("span");
            prefix.classList.add("text-muted");
            prefix.textContent = entityTypeRaw ? `${entityTypeRaw}: ` : "";
            tdEntity.appendChild(prefix);

            const mkLink = (href, text, title = "") => {
                const a = document.createElement("a");
                a.href = href;
                a.textContent = text;
                a.classList.add("redactable");
                if (title) a.title = title;
                return a;
            };

            if (entityType === "circuit" && entityId) {
                const parsed = parseCircuitEntityId(entityId);
                const circuitId = parsed.circuitId;
                let display = parsed.display;
                let title = parsed.hasName ? circuitId : "";
                tdEntity.appendChild(
                    mkLink(`circuit.html?id=${encodeURIComponent(circuitId)}`, display, title),
                );
            } else if (entityType === "node" && entityId) {
                const nodeId = this.nodeIdByName.get(entityId);
                if (nodeId !== undefined && nodeId !== null) {
                    tdEntity.appendChild(
                        mkLink(
                            `tree.html?parent=${encodeURIComponent(String(nodeId))}`,
                            entityId,
                            `Node ID: ${nodeId}`,
                        ),
                    );
                } else {
                    const span = document.createElement("span");
                    span.textContent = entityId;
                    span.classList.add("redactable");
                    tdEntity.appendChild(span);
                }
            } else {
                const span = document.createElement("span");
                span.textContent = entityId || entityTypeRaw || "";
                span.classList.add("redactable");
                tdEntity.appendChild(span);
            }

            const tdAction = document.createElement("td");
            const action = renderAction(entry.action);
            tdAction.title = action.raw;
            tdAction.appendChild(mkIcon(action.iconClass, action.iconExtra));
            const actionText = document.createElement("span");
            actionText.textContent = ` ${action.label}`;
            tdAction.appendChild(actionText);

            const tdOutcome = document.createElement("td");
            const outcome = classifyOutcome(entry, action);
            tdOutcome.appendChild(mkBadge(outcome.label, outcome.className, entry.reason || ""));
            if (outcome.detail) {
                tdOutcome.appendChild(document.createTextNode(" "));
                tdOutcome.appendChild(outcome.detail);
            }

            const tdReason = document.createElement("td");
            const reason = formatReason(entry.reason);
            tdReason.textContent = reason.label;
            if (reason.title) tdReason.title = reason.title;

            tr.appendChild(tdTime);
            tr.appendChild(tdEntity);
            tr.appendChild(tdAction);
            tr.appendChild(tdOutcome);
            tr.appendChild(tdReason);
            this.tbody.appendChild(tr);
        });
        this.renderViewMode();
    }
}
