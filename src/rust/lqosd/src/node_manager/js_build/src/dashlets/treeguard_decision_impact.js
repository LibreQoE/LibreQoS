import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {redactCell} from "../helpers/redact";

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

function statCard(label, value, toneClass = "text-primary") {
    const wrap = document.createElement("div");
    wrap.classList.add(
        "d-flex",
        "align-items-center",
        "justify-content-between",
        "gap-2",
        "border",
        "rounded",
        "px-2",
        "py-1",
        "bg-body-tertiary",
        "small",
    );

    const left = document.createElement("div");
    left.classList.add("text-body-secondary");
    left.textContent = label;

    const right = document.createElement("div");
    right.classList.add("fw-semibold", toneClass);
    right.textContent = value;

    wrap.appendChild(left);
    wrap.appendChild(right);
    return wrap;
}

export class TreeGuardDecisionImpactDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.status = null;
    }

    title() {
        return "Decision Impact";
    }

    tooltip() {
        return "<h5>TreeGuard Decision Impact</h5><p>Summarizes TreeGuard's current live impact and current warnings or errors without repeating the recent activity feed.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.chipsEl = document.createElement("div");
        this.chipsEl.classList.add("d-flex", "flex-wrap", "gap-2", "mb-3");

        this.statsGrid = document.createElement("div");
        this.statsGrid.classList.add("d-flex", "flex-wrap", "gap-2", "mb-3");

        this.alertSummaryEl = document.createElement("div");
        this.alertSummaryEl.classList.add("d-flex", "flex-column", "gap-1", "small");

        wrap.appendChild(this.chipsEl);
        wrap.appendChild(this.statsGrid);
        wrap.appendChild(this.alertSummaryEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardStatus") {
            return;
        }
        this.status = msg.data || null;
        this.renderDecision();
    }

    renderDecision() {
        this.chipsEl.innerHTML = "";
        this.alertSummaryEl.innerHTML = "";
        this.chipsEl.appendChild(
            chip(this.status?.dry_run ? "Dry Run" : "Live", this.status?.dry_run ? "bg-warning-subtle text-warning border border-warning-subtle" : "bg-success-subtle text-success border border-success-subtle"),
        );
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

        this.statsGrid.innerHTML = "";
        const virtualized = Number.isFinite(this.status?.virtualized_nodes) ? this.status.virtualized_nodes : 0;
        const fqCodel = Number.isFinite(this.status?.fq_codel_circuits) ? this.status.fq_codel_circuits : 0;
        const cpuMax = Number.isFinite(Number(this.status?.cpu_max_pct))
            ? `${Math.max(0, Math.trunc(Number(this.status.cpu_max_pct)))}%`
            : "N/A";
        [
            statCard("Virtualized", virtualized.toLocaleString(), virtualized > 0 ? "text-warning" : "text-body"),
            statCard("fq_codel", fqCodel.toLocaleString(), fqCodel > 0 ? "text-info" : "text-body"),
            statCard("CPU Max", cpuMax, "text-body"),
        ].forEach((card) => {
            card.style.minWidth = "calc(50% - 0.25rem)";
            this.statsGrid.appendChild(card);
        });

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
            redactCell(text);
            row.appendChild(text);

            this.alertSummaryEl.appendChild(row);
        });
        if (alertSummaryLines.length === 0) {
            this.alertSummaryEl.classList.add("d-none");
        } else {
            this.alertSummaryEl.classList.remove("d-none");
        }
    }
}
