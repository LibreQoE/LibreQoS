import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {get_ws_client} from "../pubsub/ws";
import {mkBadge} from "./bakery_shared";

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
    } else if (lowerVerb === "unvirtualize") {
        iconClass = "fa-expand";
        iconExtra = [];
        label = "Unvirtualize";
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

export class TreeGuardActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.nodeIdByName = new Map();
        this.lastEntries = [];
    }

    title() {
        return "TreeGuard Activity";
    }

    tooltip() {
        return "<h5>TreeGuard Activity</h5><p>Recent TreeGuard intents with explicit outcomes so operators can distinguish dry-runs, successful applies, cleanup-pending actions, skips, and failures.</p>";
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
        wrap.appendChild(tableWrap);
        base.appendChild(wrap);
        return base;
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
        this.tbody.innerHTML = "";

        if (entries.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 5;
            td.classList.add("text-muted");
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
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
    }
}
