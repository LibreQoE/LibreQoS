import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {formatUnixSecondsToLocalDateTime, mkBadge} from "./bakery_shared";

const BAKERY_ACTIVITY_PAGE_SIZE = 10;

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
    } else if (event === "live_change_started" || (!summaryLower.includes("processing batch") && summaryLower.includes("live"))) {
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

export class BakeryActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.entries = [];
        this.currentPage = 0;
    }

    title() {
        return "Recent Bakery Events";
    }

    tooltip() {
        return "<h5>Recent Bakery Events</h5><p>Recent Bakery lifecycle events grouped into stage, outcome, and scope so operators can see whether Bakery is planning, blocked, applying, cleaning up, or has finished.</p>";
    }

    subscribeTo() {
        return ["BakeryActivity"];
    }

    buildContainer() {
        const base = super.buildContainer();
        base.classList.add("dashbox-body-scroll");

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

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
        wrap.appendChild(tableWrap);

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
        wrap.appendChild(footer);
        base.appendChild(wrap);
        return base;
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
            tdSummary.textContent = entry?.summary || "—";
            tdSummary.title = entry?.event || "";

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
    }

    onMessage(msg) {
        if (msg.event !== "BakeryActivity") {
            return;
        }

        this.entries = Array.isArray(msg.data) ? msg.data : [];
        this.renderPage();
    }
}
