import {
    colorSwatch,
    linkToExecutiveLeaderboardRow,
    pollExecutiveLeaderboardPage,
    renderCircuitLink,
} from "./executive_utils";
import {colorByCapacity, formatLatest} from "./dashlets/executive_heatmap_shared";
import {colorByRetransmitPct, colorByRttMs} from "./helpers/color_scales";
import {toNumber} from "./lq_js_common/helpers/scaling";
import {formatTime} from "./i18n/format.js";
import {t} from "./i18n/index.js";

const DEFAULT_PAGE_SIZE = 50;
const PAGE_REFRESH_MS = 5000;
const SECONDS_PER_BLOCK = 60;
const BITS_PER_MEGABIT = 1_000_000;

function formatBytes(bytes) {
    let value = toNumber(bytes, NaN);
    if (!Number.isFinite(value) || value <= 0) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let idx = 0;
    while (value >= 1024 && idx < units.length - 1) {
        value /= 1024;
        idx += 1;
    }
    return `${value.toFixed(value >= 10 ? 1 : 2)} ${units[idx]}`;
}

function renderLinkedLabel(label, link) {
    if (!link) return `<span class="redactable">${label}</span>`;
    return `<a class="redactable" href="${link}">${label}</a>`;
}

const configs = {
    worst_sites: {
        kind: "WorstSitesByRtt",
        empty: () => t("executive.leaderboard.empty.worst_sites"),
        columns: [
            {
                header: () => t("topology.node_kind.site"),
                render: (row) => renderLinkedLabel(row.site_name, linkToExecutiveLeaderboardRow(row)),
            },
            {
                header: () => t("executive.col.median_rtt_ms"),
                render: (row) => `${colorSwatch(colorByRttMs(row.median_rtt_ms))}${row.median_rtt_ms.toFixed(1)}`,
            },
            {
                header: () => t("executive.col.avg_down_util"),
                render: (row) => row.avg_down_util === null || row.avg_down_util === undefined
                    ? "—"
                    : `${colorSwatch(colorByCapacity(row.avg_down_util))}${row.avg_down_util.toFixed(1)}`,
            },
            {
                header: () => t("executive.col.avg_up_util"),
                render: (row) => row.avg_up_util === null || row.avg_up_util === undefined
                    ? "—"
                    : `${colorSwatch(colorByCapacity(row.avg_up_util))}${row.avg_up_util.toFixed(1)}`,
            },
        ],
    },
    oversubscribed_sites: {
        kind: "OversubscribedSites",
        empty: () => t("executive.leaderboard.empty.oversubscribed"),
        columns: [
            {
                header: () => t("topology.node_kind.site"),
                render: (row) => renderLinkedLabel(row.site_name, linkToExecutiveLeaderboardRow(row)),
            },
            { header: () => t("executive.col.oversub_down"), render: (row) => row.ratio_down === null || row.ratio_down === undefined ? "—" : `${row.ratio_down.toFixed(2)}x` },
            { header: () => t("executive.col.oversub_up"), render: (row) => row.ratio_up === null || row.ratio_up === undefined ? "—" : `${row.ratio_up.toFixed(2)}x` },
            {
                header: () => t("executive.col.cap_du"),
                render: (row) => `${formatLatest(row.cap_down, "Mbps", row.cap_down >= 10 ? 1 : 2)} / ${formatLatest(row.cap_up, "Mbps", row.cap_up >= 10 ? 1 : 2)}`,
            },
            {
                header: () => t("executive.col.subscribed_du"),
                render: (row) => `${formatLatest(row.sub_down, "Mbps", row.sub_down >= 10 ? 1 : 2)} / ${formatLatest(row.sub_up, "Mbps", row.sub_up >= 10 ? 1 : 2)}`,
            },
            {
                header: () => t("executive.col.median_rtt_ms"),
                render: (row) => row.median_rtt_ms === null || row.median_rtt_ms === undefined
                    ? "—"
                    : `${colorSwatch(colorByRttMs(row.median_rtt_ms))}${formatLatest(row.median_rtt_ms, "ms", 1)}`,
            },
            {
                header: () => t("executive.col.avg_down_util"),
                render: (row) => row.avg_down_util === null || row.avg_down_util === undefined
                    ? "—"
                    : `${colorSwatch(colorByCapacity(row.avg_down_util))}${row.avg_down_util.toFixed(1)}`,
            },
            {
                header: () => t("executive.col.avg_up_util"),
                render: (row) => row.avg_up_util === null || row.avg_up_util === undefined
                    ? "—"
                    : `${colorSwatch(colorByCapacity(row.avg_up_util))}${row.avg_up_util.toFixed(1)}`,
            },
        ],
    },
    sites_due_upgrade: {
        kind: "SitesDueUpgrade",
        empty: () => t("executive.leaderboard.empty.sites_due_upgrade"),
        columns: [
            {
                header: () => t("topology.node_kind.site"),
                render: (row) => renderLinkedLabel(row.site_name, linkToExecutiveLeaderboardRow(row)),
            },
            {
                header: () => t("executive.col.avg_down_util"),
                render: (row) => row.avg_down_util.toFixed(1),
            },
            {
                header: () => t("executive.col.avg_up_util"),
                render: (row) => row.avg_up_util.toFixed(1),
            },
        ],
    },
    circuits_due_upgrade: {
        kind: "CircuitsDueUpgrade",
        empty: () => t("executive.leaderboard.empty.circuits_due_upgrade"),
        columns: [
            {
                header: () => t("common.table.circuit"),
                render: (row) => renderCircuitLink(row.circuit_name, row.circuit_id),
            },
            {
                header: () => t("executive.col.avg_down_util"),
                render: (row) => row.avg_down_util.toFixed(1),
            },
            {
                header: () => t("executive.col.avg_up_util"),
                render: (row) => row.avg_up_util.toFixed(1),
            },
        ],
    },
    top_asns: {
        kind: "TopAsnsByTraffic",
        empty: () => t("asn_analysis.no_heatmap_data"),
        columns: [
            {
                header: () => t("executive.col.asn"),
                render: (row) => row.asn_name
                    ? t("asn_analysis.label_with_asn", {name: row.asn_name, asn: String(row.asn)})
                    : t("executive.leaderboard.asn_number", {asn: String(row.asn)}),
            },
            {
                header: () => t("executive.col.total_traffic_15m"),
                render: (row) => formatBytes(row.total_bytes_15m || 0),
            },
            {
                header: () => t("executive.col.median_rtt_ms"),
                render: (row) => row.median_rtt_ms === null || row.median_rtt_ms === undefined
                    ? "—"
                    : `${colorSwatch(colorByRttMs(row.median_rtt_ms))}${row.median_rtt_ms.toFixed(1)}`,
            },
            {
                header: () => t("executive.col.median_retrans"),
                render: (row) => row.median_retransmit_pct === null || row.median_retransmit_pct === undefined
                    ? "—"
                    : `${colorSwatch(colorByRetransmitPct(Math.min(10, Math.max(0, row.median_retransmit_pct))))}${row.median_retransmit_pct.toFixed(2)}`,
            },
        ],
    },
};

// translatable: false — 'data-exec-page=" "' is the pagination attribute the two buttons carry, not a label
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

function tableMarkup(columns, rows, emptyMessage) {
    if (!rows.length) {
        return `<div class="text-muted small">${emptyMessage}</div>`;
    }
    const thead = `<thead><tr>${columns.map((column) => `<th scope="col">${column.header()}</th>`).join("")}</tr></thead>`;
    const tbody = rows.map((row) => `<tr>${columns.map((column) => `<td>${column.render(row)}</td>`).join("")}</tr>`).join("");
    return `
        <div class="table-responsive lqos-table-wrap">
            <table class="lqos-table lqos-table-compact align-middle mb-0">
                ${thead}
                <tbody>${tbody}</tbody>
            </table>
        </div>
    `;
}

export function renderExecutiveLeaderboardPage(targetId, configKey) {
    const cfg = configs[configKey];
    const target = document.getElementById(targetId);
    if (!cfg || !target) return;

    const state = {
        query: {
            kind: cfg.kind,
            page: 0,
            page_size: DEFAULT_PAGE_SIZE,
        },
        pollHandle: null,
    };

    const renderRows = (data) => {
        const pageSize = Number(data?.query?.page_size ?? state.query.page_size ?? DEFAULT_PAGE_SIZE);
        const page = Number(data?.query?.page ?? state.query.page ?? 0);
        const totalRows = Number(data?.total_rows ?? 0);
        const totalPages = Math.max(1, Math.ceil(totalRows / Math.max(1, pageSize)));
        const hasPrev = page > 0;
        const hasNext = page + 1 < totalPages;
        const generatedAt = toNumber(data?.generated_at_unix_ms, Date.now());
        target.innerHTML = `
            <div class="d-flex align-items-center justify-content-end flex-wrap gap-2 mb-2">
                <span class="small text-muted">${t("common.state.updated", {time: formatTime(generatedAt)})}</span>
                <button class="btn btn-sm btn-outline-secondary" ${hasPrev ? `data-exec-page="${page - 1}"` : "disabled"}>${t("common.page.prev")}</button>
                <span class="small text-muted">${t("common.page.position", {page: page + 1, pages: totalPages})}</span>
                <button class="btn btn-sm btn-outline-secondary" ${hasNext ? `data-exec-page="${page + 1}"` : "disabled"}>${t("common.page.next")}</button>
            </div>
            ${tableMarkup(cfg.columns, data?.rows || [], cfg.empty())}
        `;
        attachPaginationHandlers(target, state);
    };

    state.pollHandle = pollExecutiveLeaderboardPage(state.query, renderRows, PAGE_REFRESH_MS);
    target.innerHTML = `<div class="text-muted small">${t("executive.page.waiting_executive")}</div>`;
}
