import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {bakeryModeBadge, bakeryPreflightBadge, formatDurationMs, formatElapsedSince, mkBadge} from "./bakery_shared";

function stageClasses(kind) {
    switch (kind) {
        case "active":
            return ["border-primary-subtle", "bg-primary-subtle", "text-primary"];
        case "ok":
            return ["border-success-subtle", "bg-success-subtle", "text-success"];
        case "warning":
            return ["border-warning-subtle", "bg-warning-subtle", "text-warning"];
        case "danger":
            return ["border-danger-subtle", "bg-danger-subtle", "text-danger"];
        default:
            return ["border-secondary-subtle", "bg-body-tertiary", "text-body-secondary"];
    }
}

function renderStage(host, iconClass, title, statusText, tone, footerNode = null, titleNode = null) {
    host.innerHTML = "";
    host.className = "";
    host.classList.add(
        "bakery-pipeline-stage",
        "border",
        "rounded",
        "d-flex",
        "flex-column",
        "h-100",
        "small",
    );
    stageClasses(tone).forEach((cls) => host.classList.add(cls));

    const top = document.createElement("div");
    top.classList.add("bakery-pipeline-stage-top", "d-flex", "align-items-center", "gap-2");

    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconClass, "bakery-pipeline-stage-icon");
    top.appendChild(icon);

    const titleWrap = document.createElement("div");
    titleWrap.classList.add("bakery-pipeline-stage-title", "fw-semibold");
    if (titleNode) {
        titleWrap.appendChild(titleNode);
    } else {
        titleWrap.textContent = title;
    }
    top.appendChild(titleWrap);
    host.appendChild(top);

    const status = document.createElement("div");
    status.classList.add("bakery-pipeline-stage-status", "fw-semibold");
    status.textContent = statusText;
    status.title = statusText;
    host.appendChild(status);

    if (footerNode) {
        const footer = document.createElement("div");
        footer.classList.add("bakery-pipeline-stage-footer", "text-body-secondary");
        footer.appendChild(footerNode);
        footer.title = footer.textContent || "";
        host.appendChild(footer);
    }
}

export class BakeryPipelineDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.lastStatus = null;
    }

    title() {
        return "Pipeline";
    }

    tooltip() {
        return "<h5>Bakery Pipeline</h5><p>Visual overview of Bakery's queue-control flow: plan, preflight, build, apply, and verify.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const layout = document.createElement("div");
        layout.classList.add("bakery-pipeline-layout");
        const topRow = document.createElement("div");
        topRow.classList.add("bakery-pipeline-row", "bakery-pipeline-row-top");
        const bottomRow = document.createElement("div");
        bottomRow.classList.add("bakery-pipeline-row", "bakery-pipeline-row-bottom");
        this.stageEls = [];
        for (let i = 0; i < 3; i++) {
            const col = document.createElement("div");
            col.classList.add("bakery-pipeline-cell");
            const stage = document.createElement("div");
            col.appendChild(stage);
            topRow.appendChild(col);
            this.stageEls.push(stage);
        }
        for (let i = 0; i < 3; i++) {
            const col = document.createElement("div");
            col.classList.add("bakery-pipeline-cell");
            const stage = document.createElement("div");
            col.appendChild(stage);
            bottomRow.appendChild(col);
            this.stageEls.push(stage);
        }
        layout.appendChild(topRow);
        layout.appendChild(bottomRow);

        this.progressWrap = document.createElement("div");
        this.progressWrap.classList.add("mt-3");

        this.alertEl = document.createElement("div");
        this.alertEl.classList.add("alert", "alert-danger", "small", "py-2", "px-3", "mt-3", "d-none");

        const progressHeader = document.createElement("div");
        progressHeader.classList.add("d-flex", "justify-content-between", "align-items-center", "small", "mb-1");
        this.progressSummaryEl = document.createElement("span");
        this.progressSummaryEl.classList.add("text-body-secondary");
        this.progressPercentEl = document.createElement("span");
        this.progressPercentEl.classList.add("fw-semibold");
        progressHeader.appendChild(this.progressSummaryEl);
        progressHeader.appendChild(this.progressPercentEl);

        this.progressBarWrapEl = document.createElement("div");
        this.progressBarWrapEl.classList.add("progress");
        this.progressBarWrapEl.style.height = "0.9rem";
        this.progressBarEl = document.createElement("div");
        this.progressBarEl.classList.add("progress-bar");
        this.progressBarEl.setAttribute("role", "progressbar");
        this.progressBarEl.style.width = "0%";
        this.progressBarEl.textContent = "0%";
        this.progressBarWrapEl.appendChild(this.progressBarEl);

        this.progressWrap.appendChild(progressHeader);
        this.progressWrap.appendChild(this.progressBarWrapEl);

        this.footerEl = document.createElement("div");
        this.footerEl.classList.add("small", "text-body-secondary", "mt-2");

        wrap.appendChild(layout);
        wrap.appendChild(this.alertEl);
        wrap.appendChild(this.progressWrap);
        wrap.appendChild(this.footerEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "BakeryStatus") return;
        this.lastStatus = msg?.data?.currentState || null;
        this.renderStatus(this.lastStatus);
    }

    renderStatus(status = {}) {
        const preflight = status.preflight || null;
        const applying = status.mode === "ApplyingFullReload" || status.mode === "ApplyingLiveChange";
        const lastFailureUnix = Number.isFinite(status.lastFailureUnix) ? status.lastFailureUnix : 0;
        const lastSuccessUnix = Number.isFinite(status.lastSuccessUnix) ? status.lastSuccessUnix : 0;
        const lastOutcomeFailed = lastFailureUnix > lastSuccessUnix;
        const reloadRequired = !!status.reloadRequired;
        const reloadRequiredReason = (status.reloadRequiredReason || "").toString().trim();
        const totalCommands = Number.isFinite(status.currentApplyTotalTcCommands) ? status.currentApplyTotalTcCommands : 0;
        const completedCommands = Number.isFinite(status.currentApplyCompletedTcCommands) ? status.currentApplyCompletedTcCommands : 0;
        const totalChunks = Number.isFinite(status.currentApplyTotalChunks) ? status.currentApplyTotalChunks : 0;
        const completedChunks = Number.isFinite(status.currentApplyCompletedChunks) ? status.currentApplyCompletedChunks : 0;
        const progressPct = totalCommands > 0 ? Math.max(0, Math.min(100, (completedCommands / totalCommands) * 100)) : 0;

        const planFooter = document.createElement("span");
        planFooter.textContent = Number.isFinite(status.activeCircuits)
            ? `${status.activeCircuits.toLocaleString()} active circuits`
            : "Waiting for plan";
        renderStage(
            this.stageEls[0],
            "fa-sitemap",
            "Plan",
            applying ? "Queued model ready" : "Ready",
            applying ? "active" : "ok",
            planFooter,
        );

        renderStage(
            this.stageEls[1],
            "fa-shield-halved",
            "Preflight",
            preflight ? (preflight.ok ? "Within budget" : "Blocked") : "Unknown",
            !preflight ? "idle" : (preflight.ok ? "ok" : "danger"),
            preflight ? bakeryPreflightBadge(preflight) : mkBadge("No snapshot", "bg-light text-secondary border"),
        );

        const buildFooter = document.createElement("span");
        buildFooter.textContent = formatElapsedSince(status.lastFullReloadSuccessUnix);
        renderStage(
            this.stageEls[2],
            "fa-cubes",
            "Build",
            applying ? (status.currentApplyPhase || "Preparing commands") : "Last build",
            applying ? "active" : (status.lastFullReloadSuccessUnix ? "ok" : "idle"),
            buildFooter,
        );

        const applyFooter = document.createElement("span");
        applyFooter.textContent = applying
            ? `${completedCommands.toLocaleString()} / ${totalCommands.toLocaleString()} tc`
            : formatDurationMs(status.lastApplyDurationMs);
        renderStage(
            this.stageEls[3],
            "fa-bolt",
            "Apply",
            applying ? `${progressPct.toFixed(0)}%` : (status.lastApplyType === "None" ? "Idle" : status.lastApplyType),
            applying ? "active" : (status.lastApplyType === "None" ? "idle" : "ok"),
            applyFooter,
        );

        const verifyTitleNode = bakeryModeBadge(status.mode);
        const verifyFooter = document.createElement("span");
        if (applying && status.currentActionStartedUnix) {
            verifyFooter.textContent = `Running ${formatElapsedSince(status.currentActionStartedUnix)}`;
        } else if (lastFailureUnix || lastSuccessUnix) {
            verifyFooter.textContent = lastOutcomeFailed
                ? (status.lastFailureSummary || "Last run failed")
                : "Last run verified";
        } else {
            verifyFooter.textContent = "No completed apply yet";
        }
        renderStage(
            this.stageEls[4],
            reloadRequired || lastOutcomeFailed ? "fa-circle-exclamation" : "fa-circle-check",
            "Verify",
            reloadRequired
                ? "Reload Required"
                : (applying ? "In progress" : (lastOutcomeFailed ? "Failed" : (lastSuccessUnix ? "Success" : "Pending"))),
            reloadRequired
                ? "danger"
                : (applying ? "warning" : (lastOutcomeFailed ? "danger" : (lastSuccessUnix ? "ok" : "idle"))),
            verifyFooter,
            verifyTitleNode,
        );

        const tcIoFooter = document.createElement("span");
        if (Number.isFinite(status.lastTcIoUnix) && status.lastTcIoUnix > 0) {
            tcIoFooter.textContent = `Last tc I/O ${formatElapsedSince(status.lastTcIoUnix)}`;
        } else {
            tcIoFooter.textContent = "Tracks real tc reads/writes";
        }
        const tcIoSamples = Number.isFinite(status.tcIoIntervalSamples) ? status.tcIoIntervalSamples : 0;
        renderStage(
            this.stageEls[5],
            "fa-clock",
            "TC Interval",
            Number.isFinite(status.avgTcIoIntervalMs)
                ? formatDurationMs(status.avgTcIoIntervalMs)
                : (Number.isFinite(status.lastTcIoUnix) && status.lastTcIoUnix > 0 ? "Collecting" : "No samples"),
            tcIoSamples > 0 ? "ok" : "idle",
            tcIoFooter,
        );

        this.alertEl.classList.toggle("d-none", !reloadRequired);
        this.alertEl.textContent = reloadRequired
            ? `Incremental topology mutations are frozen until Bakery performs a structural full reload. ${reloadRequiredReason}`.trim()
            : "";

        const activeFullReload = status.mode === "ApplyingFullReload" && totalCommands > 0;
        const activeLiveChange = status.mode === "ApplyingLiveChange" && totalCommands > 0;
        const progressTone = activeFullReload ? "bg-warning" : (activeLiveChange ? "bg-info" : "bg-secondary");
        this.progressSummaryEl.textContent = activeFullReload
            ? `${status.currentApplyPhase || "Applying tc command chunks"} • ${completedCommands.toLocaleString()} / ${totalCommands.toLocaleString()} tc • chunk ${Math.min(completedChunks + 1, totalChunks).toLocaleString()} / ${totalChunks.toLocaleString()}`
            : (activeLiveChange
                ? `${status.currentApplyPhase || "Applying live change"} • ${completedCommands.toLocaleString()} / ${totalCommands.toLocaleString()} tc`
                : "No apply currently running");
        this.progressPercentEl.textContent = totalCommands > 0 && applying ? `${progressPct.toFixed(1)}%` : "Idle";
        this.progressBarEl.style.width = applying ? `${progressPct}%` : "0%";
        this.progressBarEl.textContent = applying ? `${progressPct.toFixed(1)}%` : "0%";
        this.progressBarEl.className = applying
            ? `progress-bar progress-bar-striped progress-bar-animated ${progressTone}`
            : "progress-bar bg-secondary";
        this.progressBarEl.setAttribute("aria-valuenow", progressPct.toFixed(1));
        this.progressBarEl.setAttribute("aria-valuemin", "0");
        this.progressBarEl.setAttribute("aria-valuemax", "100");

        this.footerEl.textContent = reloadRequired
            ? "Bakery detected material runtime drift. A structural full reload is required before further incremental topology mutation."
            : (applying
                ? `Bakery is actively moving through the queue pipeline. Current apply type: ${status.mode === "ApplyingFullReload" ? "full reload" : "live change"}.`
                : `Last recorded apply: ${status.lastApplyType || "None"}.`);
    }
}
