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

function statusBadgeClass(status) {
    switch (status) {
        case "Submitted":
            return "bg-primary-subtle text-primary border border-primary-subtle";
        case "Applying":
            return "bg-info-subtle text-info border border-info-subtle";
        case "Deferred":
        case "AppliedAwaitingCleanup":
            return "bg-warning-subtle text-warning border border-warning-subtle";
        case "Dirty":
        case "Failed":
            return "bg-danger-subtle text-danger border border-danger-subtle";
        case "Completed":
            return "bg-success-subtle text-success border border-success-subtle";
        default:
            return "bg-light text-secondary border";
    }
}

function siteLabelFor(latest) {
    const name = (latest?.siteName ?? "").toString().trim();
    return name || `site ${latest?.siteHash ?? "?"}`;
}

function actionWords(action) {
    if ((action ?? "").toString().trim() === "Restore") {
        return {
            base: "Restore",
            progressive: "Restoring",
            completed: "Restored",
        };
    }

    return {
        base: "Virtualize",
        progressive: "Virtualizing",
        completed: "Virtualized",
    };
}

function latestActionLabel(latest) {
    const words = actionWords(latest?.action);
    const site = siteLabelFor(latest);
    switch (latest?.status) {
        case "Completed":
            return `${words.completed} ${site}`;
        case "Applying":
        case "AppliedAwaitingCleanup":
            return `${words.progressive} ${site}`;
        case "Deferred":
            return `${words.base} deferred for ${site}`;
        case "Submitted":
            return `${words.base} requested for ${site}`;
        case "Failed":
            return `${words.base} failed for ${site}`;
        case "Dirty":
            return `${words.base} needs reload for ${site}`;
        default:
            return `${words.base} ${site}`;
    }
}

function statCard(label, value, tone = "text-primary") {
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
    );

    const top = document.createElement("div");
    top.classList.add("small", "text-body-secondary", "text-truncate");
    top.textContent = label;

    const bottom = document.createElement("div");
    bottom.classList.add("fw-semibold", tone, "small");
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
        return "Live Topology Changes";
    }

    tooltip() {
        return "<h5>Live Topology Changes</h5><p>Shows human-readable TreeGuard and Bakery runtime changes, including queued requests, active virtualization work, deferred cleanup, retryable failures, structural blocks, and whether incremental topology changes are currently frozen.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.summaryGrid = document.createElement("div");
        this.summaryGrid.classList.add("d-flex", "flex-wrap", "gap-2", "mb-2");

        this.latestWrap = document.createElement("div");
        this.latestWrap.classList.add("border", "rounded", "px-2", "py-2", "mb-2");

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
        const blocked = runtime?.blockedCount || 0;
        const dirty = runtime?.dirtyCount || 0;
        const latest = runtime?.latest || null;

        this.summaryGrid.innerHTML = "";
        [
            statCard("Submitted", compactCount(submitted), submitted > 0 ? "text-primary" : "text-body"),
            statCard("Applying", compactCount(applying), applying > 0 ? "text-info" : "text-body"),
            statCard("Deferred", compactCount(deferred), deferred > 0 ? "text-warning" : "text-body"),
            statCard("Cleanup", compactCount(cleanup), cleanup > 0 ? "text-warning" : "text-body"),
            statCard("Failed", compactCount(failed), failed > 0 ? "text-danger" : "text-body"),
            statCard("Blocked", compactCount(blocked), blocked > 0 ? "text-warning" : "text-body"),
            statCard("Dirty", compactCount(dirty), dirty > 0 ? "text-danger" : "text-body"),
        ].forEach((card) => {
            card.style.minWidth = "calc(50% - 0.25rem)";
            this.summaryGrid.appendChild(card);
        });

        this.badgeWrap.innerHTML = "";
        if (status?.reloadRequired) {
            this.badgeWrap.appendChild(
                mkBadge("Reload Required", "bg-danger-subtle text-danger border border-danger-subtle", status?.reloadRequiredReason || ""),
            );
        }

        if (latest?.status) {
            this.badgeWrap.appendChild(
                mkBadge(latest.status, statusBadgeClass(latest.status), latest?.lastError || latestActionLabel(latest)),
            );
        } else if (!status?.reloadRequired && applying + cleanup + failed + blocked + dirty + deferred + submitted === 0) {
            this.badgeWrap.appendChild(mkBadge("Idle", "bg-light text-secondary border"));
        }

        if (latest) {
            this.latestMain.className = `fw-semibold mb-1 ${statusTone(latest.status)}`;
            this.latestMain.textContent = latestActionLabel(latest);

            const meta = [];
            meta.push(`Op ${latest.operationId}`);
            if (!latest?.siteName) {
                meta.push(`site hash ${latest.siteHash}`);
            }
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
        } else if (blocked > 0) {
            this.footerEl.textContent = `${compactCount(blocked)} runtime operations are structurally blocked and will retry only after the relevant topology changes.`;
        } else if (failed > 0) {
            this.footerEl.textContent = `${compactCount(failed)} runtime operations failed and may need operator attention or a full reload.`;
        } else if (deferred > 0) {
            this.footerEl.textContent = `${compactCount(deferred)} runtime operations were deferred and will retry later.`;
        } else if (cleanup > 0) {
            this.footerEl.textContent = `${compactCount(cleanup)} runtime operations are waiting for deferred Bakery cleanup.`;
        } else if (applying > 0 || submitted > 0) {
            this.footerEl.textContent = "Bakery is processing live topology mutations from TreeGuard.";
        } else {
            this.footerEl.textContent = "";
        }
        this.footerEl.classList.toggle("d-none", !this.footerEl.textContent);
    }
}
