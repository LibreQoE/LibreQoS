import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {formatElapsedSince, mkBadge} from "./bakery_shared";

function compactCount(value) {
    return Number.isFinite(value) ? value.toLocaleString() : "0";
}

function formatSeconds(deltaSeconds) {
    const delta = Math.max(0, Math.floor(deltaSeconds));
    if (delta < 60) {
        return `${delta}s`;
    }
    if (delta < 3600) {
        return `${Math.floor(delta / 60)}m ${delta % 60}s`;
    }
    return `${Math.floor(delta / 3600)}h ${Math.floor((delta % 3600) / 60)}m`;
}

function formatUntil(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "—";
    }
    return formatSeconds(n - (Date.now() / 1000));
}

function statusTone(status) {
    switch (status) {
        case "Submitted":
            return "text-primary";
        case "Applying":
            return "text-info";
        case "Deferred":
            return "text-warning";
        case "AppliedAwaitingCleanup":
            return "text-warning";
        case "Dirty":
            return "text-danger";
        case "Failed":
            return "text-danger";
        case "Completed":
            return "text-success";
        default:
            return "text-primary";
    }
}

function statCard(label, value, tone = "text-primary") {
    const wrap = document.createElement("div");
    wrap.classList.add("border", "rounded", "p-2", "bg-body-tertiary", "h-100");

    const top = document.createElement("div");
    top.classList.add("small", "text-body-secondary");
    top.textContent = label;

    const bottom = document.createElement("div");
    bottom.classList.add("fw-semibold", tone);
    bottom.textContent = value;

    wrap.appendChild(top);
    wrap.appendChild(bottom);
    return wrap;
}

export class BakeryChangeMixDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
    }

    title() {
        return "Runtime Operations";
    }

    tooltip() {
        return "<h5>Bakery Runtime Operations</h5><p>Shows live TreeGuard/Bakery runtime mutations, including queued requests, active virtualization work, deferred/backed-off operations, deferred cleanup, failures, and whether incremental topology changes are currently frozen.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.summaryGrid = document.createElement("div");
        this.summaryGrid.classList.add("row", "g-2", "mb-3");

        this.latestWrap = document.createElement("div");
        this.latestWrap.classList.add("border", "rounded", "p-2", "mb-3");

        this.badgeWrap = document.createElement("div");
        this.badgeWrap.classList.add("d-flex", "flex-wrap", "gap-2", "mb-2");

        this.latestMain = document.createElement("div");
        this.latestMain.classList.add("fw-semibold", "mb-1");

        this.latestMeta = document.createElement("div");
        this.latestMeta.classList.add("small", "text-body-secondary");

        this.latestWrap.appendChild(this.badgeWrap);
        this.latestWrap.appendChild(this.latestMain);
        this.latestWrap.appendChild(this.latestMeta);

        this.footerEl = document.createElement("div");
        this.footerEl.classList.add("small", "text-body-secondary");

        wrap.appendChild(this.summaryGrid);
        wrap.appendChild(this.latestWrap);
        wrap.appendChild(this.footerEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "BakeryStatus") return;
        const status = msg?.data?.currentState || {};
        const runtime = status.runtimeOperations || {};
        this.render(status, runtime);
    }

    render(status, runtime) {
        const submitted = runtime?.submittedCount || 0;
        const deferred = runtime?.deferredCount || 0;
        const applying = runtime?.applyingCount || 0;
        const cleanup = runtime?.awaitingCleanupCount || 0;
        const failed = runtime?.failedCount || 0;
        const dirty = runtime?.dirtyCount || 0;
        const latest = runtime?.latest || null;

        this.summaryGrid.innerHTML = "";
        [
            statCard("Submitted", compactCount(submitted), submitted > 0 ? "text-primary" : "text-body"),
            statCard("Applying", compactCount(applying), applying > 0 ? "text-info" : "text-body"),
            statCard("Deferred", compactCount(deferred), deferred > 0 ? "text-warning" : "text-body"),
            statCard("Cleanup", compactCount(cleanup), cleanup > 0 ? "text-warning" : "text-body"),
            statCard("Failed", compactCount(failed), failed > 0 ? "text-danger" : "text-body"),
            statCard("Dirty", compactCount(dirty), dirty > 0 ? "text-danger" : "text-body"),
        ].forEach((card) => {
            const col = document.createElement("div");
            col.classList.add("col-6");
            col.appendChild(card);
            this.summaryGrid.appendChild(col);
        });

        this.badgeWrap.innerHTML = "";
        if (status?.reloadRequired) {
            this.badgeWrap.appendChild(
                mkBadge("Reload Required", "bg-danger-subtle text-danger border border-danger-subtle", status?.reloadRequiredReason || ""),
            );
        } else if (applying + cleanup + failed + dirty + deferred + submitted === 0) {
            this.badgeWrap.appendChild(mkBadge("Idle", "bg-light text-secondary border"));
        }

        if (latest) {
            this.latestMain.className = `fw-semibold mb-1 ${statusTone(latest.status)}`;
            this.latestMain.textContent = `${latest.action} site ${latest.siteHash} • ${latest.status}`;

            const meta = [];
            meta.push(`Op ${latest.operationId}`);
            meta.push(`updated ${formatElapsedSince(latest.updatedAtUnix)} ago`);
            if (Number.isFinite(latest.attemptCount) && latest.attemptCount > 1) {
                meta.push(`${latest.attemptCount} attempts`);
            }
            if (latest.nextRetryAtUnix) {
                meta.push(`retry in ${formatUntil(latest.nextRetryAtUnix)}`);
            }
            if (latest.lastError) {
                meta.push(latest.lastError);
            }
            this.latestMeta.textContent = meta.join(" • ");
        } else {
            this.latestMain.className = "fw-semibold mb-1";
            this.latestMain.textContent = "No runtime Bakery operations";
            this.latestMeta.textContent = "TreeGuard has no live virtualization or restore work in flight.";
        }

        if (status?.reloadRequiredReason) {
            this.footerEl.textContent = status.reloadRequiredReason;
        } else if (dirty > 0) {
            this.footerEl.textContent = `${compactCount(dirty)} runtime subtree operations are marked dirty.`;
        } else if (failed > 0) {
            this.footerEl.textContent = `${compactCount(failed)} runtime operations failed and may need operator attention or a full reload.`;
        } else if (deferred > 0) {
            this.footerEl.textContent = `${compactCount(deferred)} runtime operations were deferred and will retry later.`;
        } else if (cleanup > 0) {
            this.footerEl.textContent = `${compactCount(cleanup)} runtime operations are waiting for deferred Bakery cleanup.`;
        } else if (applying > 0 || submitted > 0) {
            this.footerEl.textContent = "Bakery is processing live topology mutations from TreeGuard.";
        } else {
            this.footerEl.textContent = "No runtime topology mutations are currently active.";
        }
    }
}
