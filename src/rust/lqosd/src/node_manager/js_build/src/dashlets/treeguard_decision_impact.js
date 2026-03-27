import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

function splitOnce(s, sep) {
    const str = (s ?? "").toString();
    const idx = str.indexOf(sep);
    if (idx === -1) return [str, ""];
    return [str.slice(0, idx), str.slice(idx + sep.length)];
}

function parseAction(actionRaw) {
    const raw = (actionRaw ?? "").toString().trim();
    const [verbRaw, payloadRaw] = splitOnce(raw, ":");
    const verb = verbRaw.trim().toLowerCase();
    const payload = payloadRaw.trim();

    if (verb === "virtualize") {
        return { icon: "fa-compress", tone: "text-warning", title: "Virtualize node", detail: payload || "TreeGuard virtualized a node" };
    }
    if (verb === "virtualize_requested") {
        return { icon: "fa-hourglass-half", tone: "text-primary", title: "Queued virtualization", detail: payload || "TreeGuard queued a node virtualization" };
    }
    if (verb === "unvirtualize") {
        return { icon: "fa-expand", tone: "text-success", title: "Restore node", detail: payload || "TreeGuard restored a node" };
    }
    if (verb === "unvirtualize_requested") {
        return { icon: "fa-hourglass-half", tone: "text-primary", title: "Queued restore", detail: payload || "TreeGuard queued a node restore" };
    }
    if (verb === "reload_success") {
        return { icon: "fa-refresh", tone: "text-success", title: "Reload success", detail: payload || "LibreQoS reload completed" };
    }
    if (verb === "reload_failed") {
        return { icon: "fa-refresh", tone: "text-danger", title: "Reload failed", detail: payload || "LibreQoS reload failed" };
    }
    if (verb === "set_sqm_override" || verb === "set_sqm_live") {
        return { icon: "fa-sliders", tone: "text-info", title: "SQM override", detail: payload || "Circuit SQM changed" };
    }
    if (verb === "would_set_sqm_override") {
        return { icon: "fa-eye", tone: "text-body-secondary", title: "Dry-run SQM", detail: payload || "Would change circuit SQM" };
    }
    return { icon: "fa-circle-info", tone: "text-body-secondary", title: "Recent decision", detail: raw || "No recent decision" };
}

function classifyOutcome(entry) {
    const action = (entry?.action ?? "").toString().trim().toLowerCase();
    const reason = (entry?.reason ?? "").toString().trim().toLowerCase();

    if (!entry) {
        return { label: "Idle", className: "bg-light text-secondary border" };
    }
    if (action.startsWith("would_")) {
        return { label: "Dry Run", className: "bg-light text-secondary border" };
    }
    if (action.endsWith("_requested")) {
        return { label: "Queued", className: "bg-primary-subtle text-primary border border-primary-subtle" };
    }
    if (action === "reload_skipped") {
        return { label: "Skipped", className: "bg-light text-secondary border" };
    }
    if (action.endsWith("_failed") || action.includes("failed")) {
        return { label: "Failed", className: "bg-danger-subtle text-danger border border-danger-subtle" };
    }
    if (reason.includes("cleanup pending") || reason.includes("awaiting cleanup")) {
        return { label: "Cleanup Pending", className: "bg-warning-subtle text-warning border border-warning-subtle" };
    }
    if (action === "dry_run_toggled") {
        return { label: "Updated", className: "bg-primary-subtle text-primary border border-primary-subtle" };
    }
    return { label: "Applied", className: "bg-success-subtle text-success border border-success-subtle" };
}

function chip(text, className) {
    const span = document.createElement("span");
    span.className = `badge ${className}`;
    span.textContent = text;
    return span;
}

function normalizeMessage(message) {
    return (message ?? "").toString().trim();
}

function isErrorMessage(message) {
    const normalized = normalizeMessage(message).toLowerCase();
    return normalized.includes("failed")
        || normalized.includes("dirty")
        || normalized.includes("reload required")
        || normalized.includes("requires full reload")
        || normalized.includes("unable to load");
}

function splitAlerts(messages) {
    const errors = [];
    const warnings = [];
    (Array.isArray(messages) ? messages : [])
        .map(normalizeMessage)
        .filter(Boolean)
        .forEach((message) => {
            if (isErrorMessage(message)) {
                errors.push(message);
            } else {
                warnings.push(message);
            }
        });
    return { errors, warnings };
}

function buildAlertTooltip(label, messages) {
    if (!messages.length) {
        return label;
    }
    return `${label}\n\n${messages.join("\n")}`;
}

function compactAlertMessage(message) {
    const normalized = normalizeMessage(message);
    if (!normalized) {
        return "";
    }

    let match = normalized.match(/^TreeGuard links: deferred (\d+) lower-value or over-budget node virtualization changes this tick\.?$/i);
    if (match) {
        const count = Number.parseInt(match[1], 10) || 0;
        return `${count} deferred link change${count === 1 ? "" : "s"}`;
    }

    match = normalized.match(/^TreeGuard links: skipped (\d+) low-value automatic node virtualization candidates this tick because the subtree was too small for its current throughput\.?$/i);
    if (match) {
        const count = Number.parseInt(match[1], 10) || 0;
        return `${count} low-value candidate${count === 1 ? "" : "s"} skipped`;
    }

    match = normalized.match(/^TreeGuard links: deferred runtime ([^ ]+) for node '([^']+)': .*$/i);
    if (match) {
        return `Deferred runtime ${match[1]} for ${match[2]}`;
    }

    return normalized;
}

function summarizeAlerts(errors, warnings) {
    const lines = [];
    if (errors[0]) {
        lines.push({
            icon: "fa-circle-exclamation",
            tone: "text-danger",
            text: compactAlertMessage(errors[0]),
            fullText: errors[0],
        });
    }
    if (warnings[0]) {
        lines.push({
            icon: "fa-triangle-exclamation",
            tone: "text-warning",
            text: compactAlertMessage(warnings[0]),
            fullText: warnings[0],
        });
    }
    if (!errors[0] && warnings[1]) {
        lines.push({
            icon: "fa-triangle-exclamation",
            tone: "text-warning",
            text: compactAlertMessage(warnings[1]),
            fullText: warnings[1],
        });
    }
    return lines;
}

export class TreeGuardDecisionImpactDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.status = null;
        this.activity = [];
    }

    title() {
        return "Decision Impact";
    }

    tooltip() {
        return "<h5>TreeGuard Decision Impact</h5><p>Highlights the latest TreeGuard intent plus its outcome, so operators can see whether the action was queued, applied, is awaiting cleanup, was skipped, or failed.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus", "TreeGuardActivity"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const hero = document.createElement("div");
        hero.classList.add("d-flex", "align-items-start", "gap-3", "mb-3");
        this.iconEl = document.createElement("i");
        this.iconEl.classList.add("fa", "fa-2x", "fa-circle-info", "text-body-secondary");
        hero.appendChild(this.iconEl);

        const heroText = document.createElement("div");
        heroText.classList.add("flex-grow-1");
        this.titleEl = document.createElement("div");
        this.titleEl.classList.add("fw-semibold");
        this.detailEl = document.createElement("div");
        this.detailEl.classList.add("small", "text-body-secondary");
        heroText.appendChild(this.titleEl);
        heroText.appendChild(this.detailEl);
        hero.appendChild(heroText);

        this.chipsEl = document.createElement("div");
        this.chipsEl.classList.add("d-flex", "flex-wrap", "gap-2", "mb-3");

        this.alertSummaryEl = document.createElement("div");
        this.alertSummaryEl.classList.add("d-flex", "flex-column", "gap-1", "small", "mb-3");

        const recentLabel = document.createElement("div");
        recentLabel.classList.add("small", "text-body-secondary", "mb-2");
        recentLabel.textContent = "Recent actions";

        this.timelineEl = document.createElement("div");
        this.timelineEl.classList.add("d-flex", "flex-column", "gap-2", "small");

        wrap.appendChild(hero);
        wrap.appendChild(this.chipsEl);
        wrap.appendChild(this.alertSummaryEl);
        wrap.appendChild(recentLabel);
        wrap.appendChild(this.timelineEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event === "TreeGuardStatus") {
            this.status = msg.data || null;
        } else if (msg.event === "TreeGuardActivity") {
            this.activity = Array.isArray(msg.data) ? msg.data : [];
        } else {
            return;
        }
        this.renderDecision();
    }

    renderDecision() {
        const latest = this.activity[0] || null;
        const parsed = latest ? parseAction(latest.action) : {
            icon: "fa-circle-info",
            tone: "text-body-secondary",
            title: "No recent decision",
            detail: "TreeGuard has not emitted a recent action.",
        };

        this.iconEl.className = `fa fa-2x ${parsed.icon} ${parsed.tone}`;
        this.titleEl.textContent = parsed.title;
        this.detailEl.textContent = latest
            ? `${(latest.entity_type || "").toString()}: ${(latest.entity_id || "").toString()}`
            : parsed.detail;
        this.detailEl.title = latest?.reason || this.status?.last_action_summary || "";

        this.chipsEl.innerHTML = "";
        this.alertSummaryEl.innerHTML = "";
        const latestOutcome = classifyOutcome(latest);
        this.chipsEl.appendChild(chip(latestOutcome.label, latestOutcome.className));
        this.chipsEl.appendChild(
            chip(this.status?.dry_run ? "Dry Run" : "Live", this.status?.dry_run ? "bg-warning-subtle text-warning border border-warning-subtle" : "bg-success-subtle text-success border border-success-subtle"),
        );
        if (latest) {
            if (latest.entity_type) {
                this.chipsEl.appendChild(
                    chip((latest.entity_type || "").toString(), "bg-info-subtle text-info border border-info-subtle"),
                );
            }
            if (!latest.persisted && !latest.action.toLowerCase().startsWith("would_")) {
                this.chipsEl.appendChild(
                    chip("Runtime", "bg-info-subtle text-info border border-info-subtle"),
                );
            }
        }
        if (this.status?.paused_for_bakery_reload) {
            const reason = (this.status?.pause_reason || "").toString().toLowerCase();
            this.chipsEl.appendChild(
                chip(
                    reason.includes("requires full reload") ? "Full Reload Required" : "Paused for Bakery",
                    reason.includes("requires full reload")
                        ? "bg-danger-subtle text-danger border border-danger-subtle"
                        : "bg-warning-subtle text-warning border border-warning-subtle",
                ),
            );
        }
        const { errors, warnings } = splitAlerts(this.status?.warnings);
        if (warnings.length > 0) {
            const warningChip = chip(
                `${warnings.length} warning${warnings.length === 1 ? "" : "s"}`,
                "bg-warning-subtle text-warning border border-warning-subtle",
            );
            warningChip.title = buildAlertTooltip("TreeGuard warnings", warnings);
            this.chipsEl.appendChild(warningChip);
        }
        if (errors.length > 0) {
            const errorChip = chip(
                `${errors.length} error${errors.length === 1 ? "" : "s"}`,
                "bg-danger-subtle text-danger border border-danger-subtle",
            );
            errorChip.title = buildAlertTooltip("TreeGuard errors", errors);
            this.chipsEl.appendChild(errorChip);
        }

        const alertSummaryLines = summarizeAlerts(errors, warnings);
        alertSummaryLines.forEach((item) => {
            const row = document.createElement("div");
            row.classList.add("d-flex", "align-items-start", "gap-2", "text-body-secondary");
            row.title = item.fullText || item.text;

            const icon = document.createElement("i");
            icon.className = `fa fa-fw ${item.icon} ${item.tone}`;
            row.appendChild(icon);

            const text = document.createElement("div");
            text.textContent = item.text;
            row.appendChild(text);

            this.alertSummaryEl.appendChild(row);
        });
        if (alertSummaryLines.length === 0) {
            this.alertSummaryEl.classList.add("d-none");
        } else {
            this.alertSummaryEl.classList.remove("d-none");
        }

        this.timelineEl.innerHTML = "";
        const entries = this.activity.slice(0, 3);
        if (entries.length === 0) {
            const empty = document.createElement("div");
            empty.classList.add("text-body-secondary");
            empty.textContent = "No recent TreeGuard actions";
            this.timelineEl.appendChild(empty);
            return;
        }

        entries.forEach((entry) => {
            const row = document.createElement("div");
            row.classList.add("d-flex", "align-items-start", "gap-2");
            const action = parseAction(entry.action);
            const icon = document.createElement("i");
            icon.className = `fa fa-fw ${action.icon} ${action.tone}`;
            row.appendChild(icon);
            const text = document.createElement("div");
            text.classList.add("text-body-secondary");
            text.textContent = `${action.title}: ${(entry.entity_id || "").toString() || action.detail}`;
            text.title = entry.reason || "";
            row.appendChild(text);
            this.timelineEl.appendChild(row);
        });
    }
}
