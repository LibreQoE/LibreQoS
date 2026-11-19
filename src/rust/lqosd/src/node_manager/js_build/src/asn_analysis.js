import {clearDiv} from "./helpers/builders";
import {colorByQoqScore, colorByRetransmitPct, colorByRttMs} from "./helpers/color_scales";
import {isDarkMode} from "./helpers/dark_mode";
import {scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {
    medianFromBlocks,
    pollExecutiveHeatmapPage,
    pollExecutiveLeaderboardPage,
    sumBlocks,
} from "./executive_utils";
import {get_ws_client} from "./pubsub/ws";
import {formatNumber} from "./i18n/format.js";
import {t} from "./i18n/index.js";
import {relativeTimeText} from "./i18n/relative_time.mjs";

const wsClient = get_ws_client();
const TOP_LABEL_COUNT = 4;
const TOP_ROW_LIMIT = 20;
const HEATMAP_BLOCK_SECONDS = 60;
const EXECUTIVE_PAGE_SIZE = 250;
const NEUTRAL_CHART_COLOR = "#94a3b8";
// Retransmit percentages keep two decimals on the rows and one on the chart's
// value axis; `Intl` percent style places the mark for the active locale.
const RETRANS_PERCENT = {style: "percent", minimumFractionDigits: 2, maximumFractionDigits: 2};
const RETRANS_AXIS_PERCENT = {style: "percent", minimumFractionDigits: 1, maximumFractionDigits: 1};
const RTT_THRESHOLDS = (() => {
    const configured = window.rttThresholds || window.rtt_thresholds || window.config?.rtt_thresholds || {};
    const greenMs = Math.max(0, Math.round(toNumber(configured.greenMs ?? configured.green_ms ?? configured.green, 0)));
    const yellowMs = Math.max(greenMs, Math.round(toNumber(configured.yellowMs ?? configured.yellow_ms ?? configured.yellow, 100)));
    const redMs = Math.max(yellowMs, Math.round(toNumber(configured.redMs ?? configured.red_ms ?? configured.red, 200)));
    return { greenMs, yellowMs, redMs };
})();

const state = {
    executiveTopAsns: null,
    executiveDownloadHeatmap: null,
    executiveRttHeatmap: null,
    executiveRetransmitHeatmap: null,
    executiveRows: [],
    selectedAsn: null,
    asnList: [],
    listCountsByAsn: new Map(),
    leaderboardSort: { key: "traffic", dir: "desc" },
    lastExecutiveUpdate: null,
    liveClockTimer: null,
    bubbleChart: null,
    bubbleChartTheme: null,
    retransmitChart: null,
    retransmitChartTheme: null,
    themeObserver: null,
    executivePollHandles: [],
};

function listenOnce(eventName, handler) {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
}

function formatVolume(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) {
        return "—";
    }
    return `${scaleNumber(bytes, 1)}B`;
}

function formatRttMs(rttMs) {
    if (!Number.isFinite(rttMs) || rttMs <= 0) {
        return "—";
    }
    // One decimal below 100 ms, whole milliseconds from there up.
    const digits = rttMs >= 100 ? 0 : 1;
    return `${formatNumber(rttMs, {minimumFractionDigits: digits, maximumFractionDigits: digits})} ms`;
}

function formatRetransPct(value) {
    // `value` is already a percentage (2.5 means 2.50 %), so the ratio Intl's
    // percent style expects is formed here and not in the sentence.
    if (!Number.isFinite(value) || value <= 0) {
        return formatNumber(0, RETRANS_PERCENT);
    }
    return formatNumber(value / 100, RETRANS_PERCENT);
}

function formatFlowCount(value) {
    if (!Number.isFinite(value) || value <= 0) {
        return "0";
    }
    return scaleNumber(value, 0);
}

function formatTrafficRateMbps(mbps) {
    if (!Number.isFinite(mbps) || mbps <= 0) {
        return "—";
    }
    // The rate keeps the scale the tooltip already used: one decimal below
    // 100 Mb/s, whole Mb/s from there up.
    const digits = mbps >= 100 ? 0 : 1;
    return t("asn_analysis.rate_avg", {
        rate: formatNumber(mbps, {minimumFractionDigits: digits, maximumFractionDigits: digits}),
    });
}

function formatAgo(date) {
    if (!(date instanceof Date)) {
        return t("asn_analysis.waiting");
    }
    const diffSeconds = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
    if (diffSeconds < 5) {
        return t("asn_analysis.updated_just_now");
    }
    // "5 sec. ago" / "hace 5 s" comes from Intl.RelativeTimeFormat; the frame
    // around it is the shared update message, as everywhere else in the UI.
    return t("common.state.updated", {time: relativeTimeText(Math.round(date.getTime() / 1000))});
}

function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
}

/** The QoO band as a catalog token; `qooStatusLabel` renders it. */
function qooStatus(score) {
    if (!Number.isFinite(score)) {
        return "unknown";
    }
    if (score >= 90) return "optimal";
    if (score >= 75) return "good";
    if (score >= 50) return "degraded";
    return "critical";
}

function qooStatusLabel(status) {
    // One literal call per branch: the extractor reads keys, it does not run code.
    switch (status) {
        case "optimal": return t("asn_analysis.qoo.optimal");
        case "good": return t("asn_analysis.qoo.good");
        case "degraded": return t("asn_analysis.qoo.degraded");
        case "critical": return t("asn_analysis.qoo.critical");
        default: return t("asn_analysis.qoo.unknown");
    }
}

// translatable: false — "rgba(148, 163, 184, 0.14)" "rgba(148, 163, 184, 0.26)" "rgba(16, 185, 129, 0.14)" "rgba(52, 211, 153, 0.24)" "rgba(59, 130, 246, 0.14)" "rgba(96, 165, 250, 0.24)" "rgba(245, 158, 11, 0.14)" "rgba(251, 191, 36, 0.24)" "rgba(239, 68, 68, 0.16)" "rgba(248, 113, 113, 0.26)" — the QoO badge's background and border colours per band, not labels
function qooBadgeClass(score) {
    if (!Number.isFinite(score)) {
        return {
            background: "rgba(148, 163, 184, 0.14)",
            color: "#94a3b8",
            borderColor: "rgba(148, 163, 184, 0.26)",
        };
    }
    if (score >= 90) {
        return {
            background: "rgba(16, 185, 129, 0.14)",
            color: "#34d399",
            borderColor: "rgba(52, 211, 153, 0.24)",
        };
    }
    if (score >= 75) {
        return {
            background: "rgba(59, 130, 246, 0.14)",
            color: "#60a5fa",
            borderColor: "rgba(96, 165, 250, 0.24)",
        };
    }
    if (score >= 50) {
        return {
            background: "rgba(245, 158, 11, 0.14)",
            color: "#fbbf24",
            borderColor: "rgba(251, 191, 36, 0.24)",
        };
    }
    return {
        background: "rgba(239, 68, 68, 0.16)",
        color: "#f87171",
        borderColor: "rgba(248, 113, 113, 0.26)",
    };
}

function arrayMedian(values) {
    const numeric = (values || [])
        .map((value) => toNumber(value, NaN))
        .filter((value) => Number.isFinite(value));
    if (!numeric.length) {
        return null;
    }
    numeric.sort((a, b) => a - b);
    const mid = Math.floor(numeric.length / 2);
    if (numeric.length % 2 === 1) {
        return numeric[mid];
    }
    return (numeric[mid - 1] + numeric[mid]) / 2;
}

function buildExecutiveRows() {
    const topAsns = (state.executiveTopAsns?.rows || [])
        .filter((row) => row?.kind === "TopAsnByTraffic");
    const downloadRows = (state.executiveDownloadHeatmap?.rows || []);
    const rttRows = (state.executiveRttHeatmap?.rows || []);
    const retransmitRows = (state.executiveRetransmitHeatmap?.rows || []);
    const downloadByAsn = new Map(downloadRows
        .map((row) => [toNumber(row.asn, null), row]));
    const rttByAsn = new Map(rttRows
        .map((row) => [toNumber(row.asn, null), row]));
    const retransmitByAsn = new Map(retransmitRows
        .map((row) => [toNumber(row.asn, null), row]));
    const countMap = new Map(
        (state.asnList || []).map((entry) => [entry.asn, toNumber(entry.count, 0)]),
    );
    const allAsns = new Set();
    downloadRows.forEach((row) => allAsns.add(toNumber(row?.asn, null)));
    rttRows.forEach((row) => allAsns.add(toNumber(row?.asn, null)));
    retransmitRows.forEach((row) => allAsns.add(toNumber(row?.asn, null)));
    topAsns.forEach((row) => allAsns.add(toNumber(row?.asn, null)));

    const baseRows = [...allAsns]
        .filter((asnNumber) => Number.isFinite(asnNumber) && asnNumber > 0)
        .map((asnNumber) => {
        const topAsn = topAsns.find((row) => toNumber(row?.asn, 0) === asnNumber) || null;
        const downloadRow = downloadByAsn.get(asnNumber);
        const rttRow = rttByAsn.get(asnNumber);
        const retransmitRow = retransmitByAsn.get(asnNumber);
        const downloadBlocks = downloadRow?.split_blocks?.download || [];
        const uploadBlocks = downloadRow?.split_blocks?.upload || [];
        const rttBlocks = rttRow?.rtt_blocks?.rtt || [];
        const retransmitBlocks = retransmitRow?.scalar_blocks?.values || [];
        const totalMbpsMinutes = sumBlocks(downloadBlocks) + sumBlocks(uploadBlocks);
        const totalBytes = toNumber(topAsn?.total_bytes_15m, 0)
            || ((totalMbpsMinutes * 1_000_000) / 8) * HEATMAP_BLOCK_SECONDS;
        const avgMbps = totalMbpsMinutes > 0
            ? totalMbpsMinutes / Math.max(1, Math.max(downloadBlocks.length, uploadBlocks.length, 15))
            : (totalBytes * 8) / (HEATMAP_BLOCK_SECONDS * 15 * 1_000_000);
        const medianRtt = medianFromBlocks(rttBlocks);
        const medianRetrans = medianFromBlocks(retransmitBlocks);
        const flowCount = countMap.get(asnNumber) || 0;
        const asnName = topAsn?.asn_name
            || downloadRow?.label
            || rttRow?.label
            || retransmitRow?.label
            || `ASN ${asnNumber}`;
        return {
            asn: asnNumber,
            asnName,
            heatmap: {
                download: downloadBlocks,
                upload: uploadBlocks,
                rtt: rttBlocks,
                retransmit: retransmitBlocks,
            },
            totalBytes,
            avgMbps,
            flowCount,
            medianRtt,
            medianRetrans,
            rttSeverity: 0,
            retransSeverity: 0,
            trafficSeverity: 0,
            impactScore: 0,
            qooScore: 0,
            qooStatus: "unknown",
        };
    });

    const maxBytes = Math.max(0, ...baseRows.map((row) => row.totalBytes));
    baseRows.forEach((row) => {
        row.trafficSeverity = maxBytes > 0 ? row.totalBytes / maxBytes : 0;
        row.rttSeverity = clamp(
            (toNumber(row.medianRtt, 0) - RTT_THRESHOLDS.greenMs) / Math.max(1, RTT_THRESHOLDS.redMs - RTT_THRESHOLDS.greenMs),
            0,
            1,
        );
        row.retransSeverity = clamp(toNumber(row.medianRetrans, 0) / 10, 0, 1);
        row.impactScore = (row.retransSeverity * 0.5) + (row.rttSeverity * 0.3) + (row.trafficSeverity * 0.2);
        row.qooScore = Math.round(clamp(100 - (row.impactScore * 100), 0, 100));
        row.qooStatus = qooStatus(row.qooScore);
    });

    state.executiveRows = baseRows;
}

function visibleAsnRows() {
    const rows = [...state.executiveRows];
    rows.sort((a, b) => {
        if (b.totalBytes !== a.totalBytes) return b.totalBytes - a.totalBytes;
        return b.impactScore - a.impactScore;
    });
    return rows.slice(0, TOP_ROW_LIMIT);
}

function leaderboardSortValue(row, key) {
    switch (key) {
    case "asn":
        return String(row.asnName || "").toLowerCase();
    case "flows":
        return toNumber(row.flowCount, 0);
    case "rtt":
        return Number.isFinite(row.medianRtt) ? row.medianRtt : null;
    case "retrans":
        return Number.isFinite(row.medianRetrans) ? row.medianRetrans : null;
    case "qoo":
        return Number.isFinite(row.qooScore) ? row.qooScore : null;
    case "traffic":
    default:
        return toNumber(row.totalBytes, 0);
    }
}

function sortedLeaderboardRows() {
    const rows = visibleAsnRows();
    const { key, dir } = state.leaderboardSort;
    rows.sort((a, b) => {
        const left = leaderboardSortValue(a, key);
        const right = leaderboardSortValue(b, key);
        if (typeof left === "string" || typeof right === "string") {
            const cmp = String(left).localeCompare(String(right));
            return dir === "asc" ? cmp : -cmp;
        }
        const leftNull = left === null || left === undefined || !Number.isFinite(left);
        const rightNull = right === null || right === undefined || !Number.isFinite(right);
        if (leftNull && rightNull) return 0;
        if (leftNull) return 1;
        if (rightNull) return -1;
        const cmp = left - right;
        if (cmp === 0) return String(a.asnName || "").localeCompare(String(b.asnName || ""));
        return dir === "asc" ? cmp : -cmp;
    });
    return rows;
}

function updateLiveClock() {
    const cadence = document.getElementById("asnAnalysisCadence");
    const lastRefresh = document.getElementById("asnAnalysisLastRefresh");
    if (cadence) {
        cadence.innerText = t("asn_analysis.live_interval");
    }
    if (lastRefresh) {
        lastRefresh.innerText = formatAgo(state.lastExecutiveUpdate);
    }
}

function ensureLiveClock() {
    if (state.liveClockTimer !== null) {
        return;
    }
    state.liveClockTimer = window.setInterval(updateLiveClock, 1000);
}

function setSelectedAsn(asn) {
    state.selectedAsn = asn;
    renderLeaderboard();
    renderSelectedAsnSummary();
    renderBubbleChart();
    renderRetransmitChart();
}

function asnDisplayName(asn) {
    const row = state.executiveRows.find((entry) => entry.asn === asn);
    if (!row) {
        const fallback = state.asnList.find((entry) => entry.asn === asn);
        return fallback
            ? t("asn_analysis.label_with_asn", {name: fallback.name, asn: String(asn)})
            : `ASN ${asn}`;
    }
    return t("asn_analysis.label_with_asn", {name: row.asnName, asn: String(row.asn)});
}


function renderLeaderboard() {
    const tbody = document.getElementById("asnAnalysisLeaderboardBody");
    if (!tbody) {
        return;
    }
    clearDiv(tbody);

    const rows = sortedLeaderboardRows();
    if (!rows.length) {
        tbody.innerHTML = `<tr><td colspan='7' class='asn-analysis-empty'>${t("asn_analysis.no_heatmap_data")}</td></tr>`;
        return;
    }

    rows.forEach((row, index) => {
        const tr = document.createElement("tr");
        tr.className = "asn-analysis-leaderboard-row";
        tr.tabIndex = 0;
        if (row.asn === state.selectedAsn) {
            tr.classList.add("is-selected");
            tr.setAttribute("aria-current", "true");
        }
        tr.addEventListener("click", () => setSelectedAsn(row.asn));
        tr.addEventListener("keydown", (event) => {
            // translatable: false — "Enter" is the keyboard event key compared against, not a label
            if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                setSelectedAsn(row.asn);
            }
        });

        const badgeStyle = qooBadgeClass(row.qooScore);
        const trafficSummary = `${formatVolume(row.totalBytes)} · ${formatTrafficRateMbps(row.avgMbps)}`;

        // translatable: false — "ASN ·" is the ASN abbreviation and the middle dot of the leaderboard row template; ASN is invariable and the dot is punctuation
        tr.innerHTML = `
            <td class="asn-analysis-leaderboard-rank"><span class="asn-analysis-rank-chip">${index + 1}</span></td>
            <td>
                <div class="asn-analysis-asn-name">
                    <strong>${escapeHtml(row.asnName)}</strong>
                    <span class="asn-analysis-asn-sub">ASN ${row.asn}</span>
                </div>
            </td>
            <td>${trafficSummary}</td>
            <td>${formatFlowCount(row.flowCount)}</td>
            <td><span style="color:${colorByRttMs(toNumber(row.medianRtt, 0))}">${formatRttMs(row.medianRtt)}</span></td>
            <td><span style="color:${colorByRetransmitPct(toNumber(row.medianRetrans, 0))}">${formatRetransPct(toNumber(row.medianRetrans, 0))}</span></td>
            <td>
                <span class="asn-analysis-qoo-badge" style="background:${badgeStyle.background};color:${badgeStyle.color};border-color:${badgeStyle.borderColor}">
                    ${qooStatusLabel(row.qooStatus)} · ${row.qooScore}
                </span>
            </td>
        `;
        tbody.appendChild(tr);
    });
}

function renderSelectedAsnSummary() {
    const title = document.getElementById("asnAnalysisDetailTitle");
    const qooBadge = document.getElementById("asnAnalysisQooBadge");
    const kpis = document.getElementById("asnAnalysisKpis");

    if (!title || !qooBadge || !kpis) {
        return;
    }

    const row = state.executiveRows.find((entry) => entry.asn === state.selectedAsn) || null;
    if (!row) {
        title.innerText = t("asn_analysis.no_asn_selected");
        qooBadge.className = "asn-analysis-qoo-badge asn-analysis-qoo-badge--empty";
        qooBadge.innerText = "—";
        qooBadge.style.background = "";
        qooBadge.style.color = "";
        qooBadge.style.borderColor = "";
        kpis.innerHTML = `
            <div class="asn-analysis-selected-metric"><label>${t("asn_analysis.metric.median_rtt")}</label><strong>—</strong></div>
            <div class="asn-analysis-selected-metric"><label>${t("asn_analysis.metric.retransmit")}</label><strong>—</strong></div>
            <div class="asn-analysis-selected-metric"><label>${t("asn_analysis.metric.recent_flows")}</label><strong>—</strong></div>
            <div class="asn-analysis-selected-metric"><label>${t("asn_analysis.metric.recent_traffic")}</label><strong>—</strong></div>
        `;
        return;
    }

    title.innerText = t("asn_analysis.label_with_asn", {name: row.asnName, asn: String(row.asn)});

    const badgeStyle = qooBadgeClass(row.qooScore);
    qooBadge.className = "asn-analysis-qoo-badge";
    qooBadge.innerText = `${qooStatusLabel(row.qooStatus)} · ${row.qooScore}`;
    qooBadge.style.background = badgeStyle.background;
    qooBadge.style.color = badgeStyle.color;
    qooBadge.style.borderColor = badgeStyle.borderColor;

    const rttValue = toNumber(row.medianRtt, 0);
    // translatable: false — "color:" is the CSS declaration prefix the metric row inlines
    const rttStyle = rttValue > 0 ? `color:${colorByRttMs(rttValue)}` : "";
    const retransStyle = `color:${colorByRetransmitPct(toNumber(row.medianRetrans, 0))}`;

    kpis.innerHTML = `
        <div class="asn-analysis-selected-metric">
            <label>${t("asn_analysis.metric.median_rtt")}</label>
            <strong style="${rttStyle}">${formatRttMs(row.medianRtt)}</strong>
        </div>
        <div class="asn-analysis-selected-metric">
            <label>${t("asn_analysis.metric.retransmit")}</label>
            <strong style="${retransStyle}">${formatRetransPct(toNumber(row.medianRetrans, 0))}</strong>
        </div>
        <div class="asn-analysis-selected-metric">
            <label>${t("asn_analysis.metric.recent_flows")}</label>
            <strong>${formatFlowCount(row.flowCount)}</strong>
        </div>
        <div class="asn-analysis-selected-metric">
            <label>${t("asn_analysis.metric.recent_traffic")}</label>
            <strong>${formatVolume(row.totalBytes)}</strong>
            <small>${formatTrafficRateMbps(row.avgMbps)}</small>
        </div>
    `;
}

function ensureChart(domId, existingChart, themeKey) {
    const dom = document.getElementById(domId);
    if (!dom || typeof echarts === "undefined") {
        return null;
    }
    const theme = isDarkMode() ? "dark" : "vintage";
    if (existingChart && state[themeKey] === theme) {
        return existingChart;
    }
    if (existingChart) {
        existingChart.dispose();
    }
    state[themeKey] = theme;
    return echarts.init(dom, theme);
}

function renderBubbleChart() {
    const dom = document.getElementById("asnAnalysisBubbleChart");
    if (!dom || typeof echarts === "undefined") {
        return;
    }

    // The chart still describes the top-20 throughput set; table sorting is
    // independent of the chart's label order.
    const rows = visibleAsnRows();
    state.bubbleChart = ensureChart("asnAnalysisBubbleChart", state.bubbleChart, "bubbleChartTheme");
    if (!state.bubbleChart) {
        return;
    }

    if (!rows.length) {
        state.bubbleChart.clear();
        state.bubbleChart.setOption({
            title: {
                text: t("asn_analysis.waiting"),
                left: "center",
                top: "middle",
                textStyle: { color: NEUTRAL_CHART_COLOR, fontSize: 14, fontWeight: 500 },
            },
        });
        return;
    }

    const topLabels = new Set(rows.slice(0, TOP_LABEL_COUNT).map((row) => row.asn));
    const seriesData = rows.map((row) => ({
        name: row.asnName,
        asn: row.asn,
        value: [Math.max(row.totalBytes, 1), toNumber(row.medianRtt, 0), Math.max(row.flowCount, 1)],
        qooScore: row.qooScore,
        medianRetrans: row.medianRetrans,
        label: {
            show: topLabels.has(row.asn),
            formatter: row.asnName,
            color: isDarkMode() ? "#e2e8f0" : "#334155",
            fontSize: 11,
            position: "top",
        },
        itemStyle: {
            color: colorByQoqScore(row.qooScore),
            opacity: row.asn === state.selectedAsn ? 0.9 : 0.55,
            borderColor: row.asn === state.selectedAsn ? (isDarkMode() ? "#ffffff" : "#0f172a") : "rgba(255,255,255,0.35)",
            borderWidth: row.asn === state.selectedAsn ? 2 : 1,
            shadowBlur: row.asn === state.selectedAsn ? 10 : 0,
            // translatable: false — "rgba(96, 165, 250, 0.35)" "rgba(0, 0, 0, 0)" — the selected bubble's glow colour and the transparent no-glow, not labels
            shadowColor: row.asn === state.selectedAsn ? "rgba(96, 165, 250, 0.35)" : "rgba(0, 0, 0, 0)",
        },
        symbolSize: bubbleSizeForFlows(row.flowCount),
    }));

    state.bubbleChart.setOption({
        animationDuration: 250,
        grid: { left: 56, right: 18, top: 18, bottom: 44 },
        tooltip: {
            trigger: "item",
            formatter: (params) => {
                const row = rows.find((entry) => entry.asn === params.data.asn);
                if (!row) return "";
                return `
                    <div class="small">
                        <div class="fw-semibold">${t("asn_analysis.label_with_asn", {name: escapeHtml(row.asnName), asn: String(row.asn)})}</div>
                        <div>${t("asn_analysis.tooltip.traffic", {volume: formatVolume(row.totalBytes), rate: formatTrafficRateMbps(row.avgMbps)})}</div>
                        <div>${t("asn_analysis.tooltip.median_rtt", {rtt: formatRttMs(row.medianRtt)})}</div>
                        <div>${t("asn_analysis.tooltip.median_retransmit", {retransmit: formatRetransPct(toNumber(row.medianRetrans, 0))})}</div>
                        <div>${t("asn_analysis.tooltip.recent_flows", {flows: formatFlowCount(row.flowCount)})}</div>
                        <div>${t("asn_analysis.tooltip.qoo", {status: qooStatusLabel(row.qooStatus), score: row.qooScore})}</div>
                    </div>
                `;
            },
        },
        xAxis: {
            type: "log",
            logBase: 10,
            min: 1e6,
            max: 2e11,
            name: t("asn_analysis.axis.traffic_15m"),
            nameGap: 28,
            axisLabel: {
                formatter: (value) => formatVolume(value),
                hideOverlap: true,
            },
        },
        yAxis: {
            type: "value",
            name: t("asn_analysis.metric.median_rtt"),
            nameGap: 18,
            axisLabel: {
                formatter: (value) => value > 0 ? `${Math.round(value)} ms` : "—",
            },
        },
        series: [{
            type: "scatter",
            data: seriesData,
            labelLayout: {
                hideOverlap: true,
            },
            emphasis: {
                focus: "series",
            },
        }],
    }, true);

    state.bubbleChart.off("click");
    state.bubbleChart.on("click", (params) => {
        if (params?.data?.asn) {
            setSelectedAsn(params.data.asn);
        }
    });
}

function bubbleSizeForFlows(flowCount) {
    const count = Math.max(1, toNumber(flowCount, 0));
    return clamp(Math.sqrt(count) * 1.6, 8, 26);
}

function renderRetransmitChart() {
    const dom = document.getElementById("asnAnalysisRetransmitChart");
    if (!dom || typeof echarts === "undefined") {
        return;
    }

    state.retransmitChart = ensureChart("asnAnalysisRetransmitChart", state.retransmitChart, "retransmitChartTheme");
    if (!state.retransmitChart) {
        return;
    }

    const rows = [...visibleAsnRows()]
        .filter((row) => toNumber(row.medianRetrans, 0) > 0)
        .sort((a, b) => {
            const retransmitDiff = toNumber(b.medianRetrans, 0) - toNumber(a.medianRetrans, 0);
            if (retransmitDiff !== 0) return retransmitDiff;
            return b.totalBytes - a.totalBytes;
        })
        .slice(0, 10);
    if (!rows.length) {
        state.retransmitChart.clear();
        state.retransmitChart.setOption({
            title: {
                text: t("asn_analysis.no_retransmits_top20"),
                left: "center",
                top: "middle",
                textStyle: { color: NEUTRAL_CHART_COLOR, fontSize: 13, fontWeight: 500 },
            },
        });
        return;
    }

    const seriesData = rows.map((row) => ({
        value: toNumber(row.medianRetrans, 0),
        asn: row.asn,
        asnName: row.asnName,
        totalBytes: row.totalBytes,
        avgMbps: row.avgMbps,
        medianRtt: row.medianRtt,
        qooScore: row.qooScore,
        itemStyle: {
            color: colorByRetransmitPct(toNumber(row.medianRetrans, 0)),
            borderColor: row.asn === state.selectedAsn ? (isDarkMode() ? "#f8fafc" : "#0f172a") : "transparent",
            borderWidth: row.asn === state.selectedAsn ? 2 : 0,
        },
    }));

    state.retransmitChart.setOption({
        animationDuration: 250,
        grid: { left: 124, right: 18, top: 14, bottom: 26 },
        tooltip: {
            trigger: "item",
            formatter: (params) => {
                const row = rows.find((entry) => entry.asn === params?.data?.asn);
                if (!row) return "";
                return `
                    <div class="small">
                        <div class="fw-semibold">${t("asn_analysis.label_with_asn", {name: escapeHtml(row.asnName), asn: String(row.asn)})}</div>
                        <div>${t("asn_analysis.tooltip.retransmit", {retransmit: formatRetransPct(toNumber(row.medianRetrans, 0))})}</div>
                        <div>${t("asn_analysis.tooltip.traffic", {volume: formatVolume(row.totalBytes), rate: formatTrafficRateMbps(row.avgMbps)})}</div>
                        <div>${t("asn_analysis.tooltip.median_rtt", {rtt: formatRttMs(row.medianRtt)})}</div>
                        <div>${t("asn_analysis.tooltip.qoo", {status: qooStatusLabel(row.qooStatus), score: row.qooScore})}</div>
                    </div>
                `;
            },
        },
        xAxis: {
            type: "value",
            axisLabel: {
                formatter: (value) => formatNumber(toNumber(value, 0) / 100, RETRANS_AXIS_PERCENT),
            },
        },
        yAxis: {
            type: "category",
            data: rows.map((row) => row.asnName),
            inverse: true,
            axisTick: { show: false },
            axisLabel: {
                width: 112,
                overflow: "truncate",
                color: isDarkMode() ? "#cbd5e1" : "#475569",
            },
        },
        series: [{
            type: "bar",
            data: seriesData,
            barWidth: "58%",
            roundCap: true,
            label: {
                show: true,
                position: "right",
                formatter: (params) => formatRetransPct(toNumber(params?.data?.value, 0)),
                color: isDarkMode() ? "#e2e8f0" : "#334155",
                fontSize: 11,
            },
        }],
    }, true);

    state.retransmitChart.off("click");
    state.retransmitChart.on("click", (params) => {
        if (params?.data?.asn) {
            setSelectedAsn(params.data.asn);
        }
    });
}

function requestLists() {
    requestAsnList();
}

function requestAsnList() {
    listenOnce("AsnList", (msg) => {
        state.asnList = (msg?.data || []).slice().sort((a, b) => toNumber(b?.count, 0) - toNumber(a?.count, 0));
        state.listCountsByAsn = new Map(state.asnList.map((row) => [row.asn, toNumber(row.count, 0)]));
        buildExecutiveRows();
        renderAll();
    });
    wsClient.send({ AsnList: {} });
}

function escapeHtml(value) {
    return String(value ?? "")
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#39;");
}

function renderAll() {
    renderLeaderboard();
    renderSelectedAsnSummary();
    renderBubbleChart();
    renderRetransmitChart();
}

function chooseDefaultSelection() {
    if (state.selectedAsn !== null) {
        const stillExists = visibleAsnRows().some((row) => row.asn === state.selectedAsn);
        if (stillExists) {
            return;
        }
    }
    const top = sortedLeaderboardRows()[0];
    if (!top) return;
    setSelectedAsn(top.asn);
}

const DEFAULT_SORT_DIR = {
    traffic: "desc",
    flows: "desc",
    qoo: "desc",
    asn: "asc",
    rtt: "asc",
    retrans: "asc",
};

function updateSortIndicators() {
    document.querySelectorAll("[data-sort-th]").forEach((th) => {
        const key = th.dataset.sortTh;
        const active = state.leaderboardSort.key === key;
        th.setAttribute("aria-sort", active
            ? (state.leaderboardSort.dir === "asc" ? "ascending" : "descending")
            : "none");
        const arrow = th.querySelector(".asn-analysis-sort-arrow");
        if (arrow) {
            arrow.textContent = active
                ? (state.leaderboardSort.dir === "asc" ? "▲" : "▼")
                : "↕";
        }
    });
}

function wireControls() {
    document.querySelectorAll("[data-asn-sort]").forEach((button) => {
        button.addEventListener("click", () => {
            const key = button.dataset.asnSort;
            const current = state.leaderboardSort;
            const dir = current.key === key
                ? (current.dir === "desc" ? "asc" : "desc")
                : (DEFAULT_SORT_DIR[key] || "desc");
            state.leaderboardSort = { key, dir };
            updateSortIndicators();
            renderLeaderboard();
            renderBubbleChart();
        });
    });

    updateSortIndicators();
}

function observeThemeAndResize() {
    window.addEventListener("resize", () => {
        state.bubbleChart?.resize();
        state.retransmitChart?.resize();
    });

    state.themeObserver = new MutationObserver((mutations) => {
        const changed = mutations.some((mutation) => mutation.attributeName === "data-bs-theme");
        if (!changed) return;
        renderBubbleChart();
        renderRetransmitChart();
    });

    state.themeObserver.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-bs-theme"],
    });
}

function initExecutiveFeed() {
    const syncExecutiveState = (key, data) => {
        state[key] = data || null;
        state.lastExecutiveUpdate = new Date(toNumber(data?.generated_at_unix_ms, Date.now()));
        buildExecutiveRows();
        chooseDefaultSelection();
        renderAll();
        updateLiveClock();
    };

    // translatable: false — "Download" "Asn" "Retransmit" "Label" — the executive heatmap request's metric, entity-kind and sort tokens the server matches, not labels
    state.executivePollHandles = [
        pollExecutiveLeaderboardPage({
            kind: "TopAsnsByTraffic",
            page: 0,
            page_size: EXECUTIVE_PAGE_SIZE,
        }, (data) => syncExecutiveState("executiveTopAsns", data)),
        pollExecutiveHeatmapPage({
            metric: "Download",
            entity_kinds: ["Asn"],
            page: 0,
            page_size: EXECUTIVE_PAGE_SIZE,
            sort: "Label",
            descending: false,
        }, (data) => syncExecutiveState("executiveDownloadHeatmap", data)),
        pollExecutiveHeatmapPage({
            metric: "Rtt",
            entity_kinds: ["Asn"],
            page: 0,
            page_size: EXECUTIVE_PAGE_SIZE,
            sort: "Label",
            descending: false,
        }, (data) => syncExecutiveState("executiveRttHeatmap", data)),
        pollExecutiveHeatmapPage({
            metric: "Retransmit",
            entity_kinds: ["Asn"],
            page: 0,
            page_size: EXECUTIVE_PAGE_SIZE,
            sort: "Label",
            descending: false,
        }, (data) => syncExecutiveState("executiveRetransmitHeatmap", data)),
    ];
}

function init() {
    wireControls();
    ensureLiveClock();
    updateLiveClock();
    observeThemeAndResize();
    requestLists();
    initExecutiveFeed();
}

document.addEventListener("DOMContentLoaded", init);
