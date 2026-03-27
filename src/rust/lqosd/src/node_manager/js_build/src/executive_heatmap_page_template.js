import {
    badgeForEntityKind,
    linkToExecutiveMetricRow,
    pollExecutiveHeatmapPage,
} from "./executive_utils";
import {
    heatRow,
    rttHeatRow,
    retransmitHeatRow,
    utilizationHeatRow,
    formatLatest,
    colorByCapacity,
} from "./dashlets/executive_heatmap_shared";
import {colorByRttMs, colorByRetransmitPct} from "./helpers/color_scales";
import {toNumber} from "./lq_js_common/helpers/scaling";

const DEFAULT_PAGE_SIZE = 50;
const PAGE_REFRESH_MS = 5000;

const metricConfigs = {
    rtt: {
        title: "RTT (p50/p90) Heatmap",
        metric: "Rtt",
        colorFn: (v) => colorByRttMs(v),
        formatFn: (v) => formatLatest(v, "ms"),
        icon: "fa-stopwatch",
        description: "Paged list of sites, circuits, and ASNs ranked server-side by latest RTT (p50/p90 quadrants).",
        legend: [
            { label: "Good", sample: 10 },
            { label: "Moderate", sample: 40 },
            { label: "Elevated", sample: 90 },
            { label: "Severe", sample: 180 },
        ],
    },
    retransmit: {
        title: "TCP Retransmits Heatmap",
        metric: "Retransmit",
        colorFn: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
        formatFn: (v) => formatLatest(v, "%", 1),
        icon: "fa-undo-alt",
        description: "Paged list ranked server-side by latest retransmit percentage.",
        legend: [
            { label: "Low", sample: 0.5 },
            { label: "Watch", sample: 1.5 },
            { label: "Elevated", sample: 3.5 },
            { label: "Severe", sample: 7.5 },
        ],
    },
    download: {
        title: "Download Utilization Heatmap",
        metric: "Download",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        icon: "fa-arrow-down",
        description: "Paged list ranked server-side by latest download utilization.",
        legend: [
            { label: "Comfortable", sample: 30 },
            { label: "Busy", sample: 70 },
            { label: "Near capacity", sample: 90 },
            { label: "Saturated", sample: 99 },
        ],
    },
    upload: {
        title: "Upload Utilization Heatmap",
        metric: "Upload",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        icon: "fa-arrow-up",
        description: "Paged list ranked server-side by latest upload utilization.",
        legend: [
            { label: "Comfortable", sample: 30 },
            { label: "Busy", sample: 70 },
            { label: "Near capacity", sample: 90 },
            { label: "Saturated", sample: 99 },
        ],
    },
};

function renderLegend(cfg) {
    const items = (cfg.legend || []).map((item) => `
        <span class="badge rounded-pill text-bg-light border d-inline-flex align-items-center gap-2">
            <span class="rounded-circle border" style="display:inline-block;width:0.8rem;height:0.8rem;background:${cfg.colorFn(item.sample)}"></span>
            <span>${item.label}</span>
        </span>
    `).join("");
    return `
        <div class="d-flex flex-wrap gap-2 align-items-center mb-3" role="note" aria-label="Heatmap legend">
            <span class="small text-muted">Legend:</span>
            ${items}
            <span class="small text-muted">Detail pages now page server-ranked rows instead of streaming the full executive universe every second.</span>
        </div>
    `;
}

function rowSplitBlocks(row) {
    return {
        download: row?.split_blocks?.download || [],
        upload: row?.split_blocks?.upload || [],
    };
}

function rowRetransmitBlocks(row) {
    return {
        retransmit: row?.scalar_blocks?.values || [],
        retransmit_down: row?.split_blocks?.download || [],
        retransmit_up: row?.split_blocks?.upload || [],
    };
}

function rowRttBlocks(row) {
    return {
        rtt: row?.rtt_blocks?.rtt || [],
        rtt_p50_down: row?.rtt_blocks?.dl_p50 || [],
        rtt_p90_down: row?.rtt_blocks?.dl_p90 || [],
        rtt_p50_up: row?.rtt_blocks?.ul_p50 || [],
        rtt_p90_up: row?.rtt_blocks?.ul_p90 || [],
    };
}

function renderMetricRow(cfg, metricKey, row) {
    const badge = badgeForEntityKind(row?.entity_kind);
    const link = linkToExecutiveMetricRow(row);
    if (metricKey === "rtt") {
        return rttHeatRow(
            row.label,
            badge,
            rowRttBlocks(row),
            cfg.colorFn,
            cfg.formatFn,
            link,
        );
    }
    if (metricKey === "retransmit") {
        return retransmitHeatRow(
            row.label,
            badge,
            rowRetransmitBlocks(row),
            cfg.colorFn,
            cfg.formatFn,
            link,
        );
    }
    return utilizationHeatRow(
        row.label,
        badge,
        rowSplitBlocks(row),
        cfg.colorFn,
        cfg.formatFn,
        link,
    );
}

function attachPaginationHandlers(target, state) {
    target.querySelectorAll("[data-exec-page]").forEach((button) => {
        button.addEventListener("click", () => {
            const nextPage = Number(button.dataset.execPage);
            if (!Number.isFinite(nextPage) || nextPage < 0 || nextPage === Number(state.query.page || 0)) {
                return;
            }
            state.query.page = nextPage;
            state.pollHandle.refresh();
        });
    });
}

function renderHeatmapTable(targetId, metricKey) {
    const cfg = metricConfigs[metricKey];
    if (!cfg) return;
    const target = document.getElementById(targetId);
    if (!target) return;

    const state = {
        query: {
            metric: cfg.metric,
            entity_kinds: ["Site", "Circuit", "Asn"],
            page: 0,
            page_size: DEFAULT_PAGE_SIZE,
            sort: "LatestValue",
            descending: true,
        },
        lastData: null,
        pollHandle: null,
    };

    const renderRows = (data) => {
        state.lastData = data;
        const pageSize = Number(data?.query?.page_size ?? state.query.page_size ?? DEFAULT_PAGE_SIZE);
        const page = Number(data?.query?.page ?? state.query.page ?? 0);
        const totalRows = Number(data?.total_rows ?? 0);
        const totalPages = Math.max(1, Math.ceil(totalRows / Math.max(1, pageSize)));
        const hasPrev = page > 0;
        const hasNext = page + 1 < totalPages;
        const generatedAt = toNumber(data?.generated_at_unix_ms, Date.now());
        const body = (data?.rows || []).map((row) => renderMetricRow(cfg, metricKey, row)).join("");

        target.innerHTML = `
            <div class="card shadow-sm">
                <div class="card-body">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0"><i class="fas ${cfg.icon} me-2 text-primary"></i>${cfg.title}</div>
                        <div class="d-flex align-items-center gap-2">
                            <span class="small text-muted">Updated ${new Date(generatedAt).toLocaleTimeString()}</span>
                            <button class="btn btn-sm btn-outline-secondary" ${hasPrev ? `data-exec-page="${page - 1}"` : "disabled"}>Prev</button>
                            <span class="small text-muted">Page ${page + 1} / ${totalPages}</span>
                            <button class="btn btn-sm btn-outline-secondary" ${hasNext ? `data-exec-page="${page + 1}"` : "disabled"}>Next</button>
                        </div>
                    </div>
                    <p class="text-muted small mb-3">${cfg.description}</p>
                    ${renderLegend(cfg)}
                    <div class="exec-heat-rows" role="list">${body || `<div class="text-muted small">No heatmap rows available.</div>`}</div>
                </div>
            </div>
        `;
        attachPaginationHandlers(target, state);
    };

    window.addEventListener("colorBlindModeChanged", () => {
        if (state.lastData) {
            renderRows(state.lastData);
        }
    });

    state.pollHandle = pollExecutiveHeatmapPage(state.query, renderRows, PAGE_REFRESH_MS);
    target.innerHTML = `<div class="text-muted small">Waiting for heatmap data…</div>`;
}

export function renderHeatmapPage(metricKey) {
    renderHeatmapTable("executiveHeatmapFull", metricKey);
}
