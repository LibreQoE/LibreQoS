import {clearDiv} from "./helpers/builders";
import {colorByQoqScore, colorByRetransmitPct, colorByRttMs} from "./helpers/color_scales";
import {isDarkMode} from "./helpers/dark_mode";
import {openFlowRttExcludeWizard} from "./lq_js_common/helpers/flow_rtt_exclude_wizard";
import {scaleNanos, scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {
    medianFromBlocks,
    pollExecutiveHeatmapPage,
    pollExecutiveLeaderboardPage,
    sumBlocks,
} from "./executive_utils";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const EVIDENCE_ITEMS_PER_PAGE = 10;
const TOP_LABEL_COUNT = 4;
const TOP_ROW_LIMIT = 20;
const HEATMAP_BLOCK_SECONDS = 60;
const EXECUTIVE_PAGE_SIZE = 250;
const LIVE_INTERVAL_TEXT = "Executive data auto-refreshes about every 5 seconds";
const NEUTRAL_CHART_COLOR = "#94a3b8";
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
    countryList: [],
    protocolList: [],
    listCountsByAsn: new Map(),
    evidenceScope: { type: "asn", value: null, label: "" },
    evidenceSort: "bytes",
    evidenceRows: [],
    evidencePage: 0,
    evidenceMinTime: Number.MAX_SAFE_INTEGER,
    evidenceMaxTime: Number.MIN_SAFE_INTEGER,
    lastExecutiveUpdate: null,
    liveClockTimer: null,
    bubbleChart: null,
    bubbleChartTheme: null,
    retransmitChart: null,
    retransmitChartTheme: null,
    themeObserver: null,
    activeEvidenceRequestToken: 0,
    executivePollHandles: [],
};

const SORT_OPTIONS = {
    start: (a, b) => toNumber(b?.start, 0) - toNumber(a?.start, 0),
    duration: (a, b) => toNumber(b?.duration_nanos, 0) - toNumber(a?.duration_nanos, 0),
    bytes: (a, b) => totalFlowBytes(b) - totalFlowBytes(a),
};

function listenOnce(eventName, handler) {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
}

function totalFlowBytes(row) {
    return toNumber(row?.total_bytes?.down, 0) + toNumber(row?.total_bytes?.up, 0);
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
    return `${rttMs.toFixed(rttMs >= 100 ? 0 : 1)} ms`;
}

function formatRetransPct(value) {
    if (!Number.isFinite(value) || value <= 0) {
        return "0.00%";
    }
    return `${value.toFixed(2)}%`;
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
    return `${mbps.toFixed(mbps >= 100 ? 0 : 1)} Mbps avg`;
}

function formatAgo(date) {
    if (!(date instanceof Date)) {
        return "Waiting for executive ASN heatmaps…";
    }
    const diffSeconds = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
    if (diffSeconds < 5) {
        return "Updated just now";
    }
    if (diffSeconds < 60) {
        return `Updated ${diffSeconds}s ago`;
    }
    const minutes = Math.floor(diffSeconds / 60);
    if (minutes < 60) {
        return `Updated ${minutes}m ago`;
    }
    const hours = Math.floor(minutes / 60);
    return `Updated ${hours}h ago`;
}

function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
}

function qooStatus(score) {
    if (!Number.isFinite(score)) {
        return "Unknown";
    }
    if (score >= 90) return "Optimal";
    if (score >= 75) return "Good";
    if (score >= 50) return "Degraded";
    return "Critical";
}

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
            qooStatus: "Unknown",
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

function sortedLeaderboardRows() {
    const rows = visibleAsnRows();
    rows.sort((a, b) => {
        if (b.totalBytes !== a.totalBytes) return b.totalBytes - a.totalBytes;
        return b.impactScore - a.impactScore;
    });
    return rows;
}

function updateLiveClock() {
    const cadence = document.getElementById("asnAnalysisCadence");
    const lastRefresh = document.getElementById("asnAnalysisLastRefresh");
    if (cadence) {
        cadence.innerText = LIVE_INTERVAL_TEXT;
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

function setSelectedAsn(asn, { resetEvidence = true } = {}) {
    state.selectedAsn = asn;
    if (resetEvidence) {
        state.evidenceScope = { type: "asn", value: asn, label: asnDisplayName(asn) };
        loadEvidenceForScope(state.evidenceScope);
    }
    renderLeaderboard();
    renderSelectedAsnSummary();
    renderBubbleChart();
    renderRetransmitChart();
}

function asnDisplayName(asn) {
    const row = state.executiveRows.find((entry) => entry.asn === asn);
    if (!row) {
        const fallback = state.asnList.find((entry) => entry.asn === asn);
        return fallback ? `${fallback.name} (ASN ${asn})` : `ASN ${asn}`;
    }
    return `${row.asnName} (ASN ${row.asn})`;
}

function rankRationale(row) {
    if (!row) {
        return "Waiting for enough data to rank ASNs.";
    }
    const reasons = [];
    if (row.retransSeverity >= row.rttSeverity && row.retransSeverity >= row.trafficSeverity) {
        reasons.push("elevated retransmit");
    }
    if (row.trafficSeverity >= row.rttSeverity) {
        reasons.push("high traffic impact");
    }
    if (row.rttSeverity > 0.25) {
        reasons.push("higher RTT");
    }
    if (!reasons.length) {
        reasons.push("balanced impact across current QoO signals");
    }
    return `Within the current top-20 traffic and throughput ASNs, this one stands out due to ${reasons.join(" + ")}.`;
}

function renderLeaderboard() {
    const tbody = document.getElementById("asnAnalysisLeaderboardBody");
    if (!tbody) {
        return;
    }
    clearDiv(tbody);

    const rows = sortedLeaderboardRows();
    if (!rows.length) {
        tbody.innerHTML = "<tr><td colspan='7' class='asn-analysis-empty'>No ASN heatmap data yet.</td></tr>";
        return;
    }

    rows.forEach((row, index) => {
        const tr = document.createElement("tr");
        tr.className = "asn-analysis-leaderboard-row";
        if (row.asn === state.selectedAsn) {
            tr.classList.add("is-selected");
        }
        tr.addEventListener("click", () => setSelectedAsn(row.asn));

        const badgeStyle = qooBadgeClass(row.qooScore);
        const trafficSummary = `${formatVolume(row.totalBytes)} · ${formatTrafficRateMbps(row.avgMbps)}`;

        tr.innerHTML = `
            <td class="asn-analysis-leaderboard-rank">${index + 1}</td>
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
                    ${row.qooStatus} · ${row.qooScore}
                </span>
            </td>
        `;
        tbody.appendChild(tr);
    });
}

function renderSelectedAsnSummary() {
    const title = document.getElementById("asnAnalysisDetailTitle");
    const subtitle = document.getElementById("asnAnalysisDetailSubtitle");
    const kpis = document.getElementById("asnAnalysisKpis");
    const rationale = document.getElementById("asnAnalysisRationale");

    if (!title || !subtitle || !kpis || !rationale) {
        return;
    }

    const row = state.executiveRows.find((entry) => entry.asn === state.selectedAsn) || null;
    if (!row) {
        title.innerText = "Selected ASN";
        subtitle.innerText = "Choose an ASN from the leaderboard or bubble chart.";
        kpis.innerHTML = `
            <div class="asn-analysis-kpi"><label>Status</label><strong>—</strong><small>Waiting for selection</small></div>
            <div class="asn-analysis-kpi"><label>Median RTT</label><strong>—</strong></div>
            <div class="asn-analysis-kpi"><label>Retransmit</label><strong>—</strong></div>
            <div class="asn-analysis-kpi"><label>Recent Flows</label><strong>—</strong></div>
            <div class="asn-analysis-kpi"><label>Recent Traffic</label><strong>—</strong></div>
        `;
        rationale.innerText = "Waiting for enough data to rank ASNs.";
        return;
    }

    title.innerText = `${row.asnName} (ASN ${row.asn})`;
    subtitle.innerText = "Selection stays pinned across live refreshes until you click Refresh or choose another ASN from the current top-20 traffic and throughput set.";

    const badgeStyle = qooBadgeClass(row.qooScore);
    kpis.innerHTML = `
        <div class="asn-analysis-kpi">
            <label>QoO Status</label>
            <strong>
                <span class="asn-analysis-qoo-badge" style="background:${badgeStyle.background};color:${badgeStyle.color};border-color:${badgeStyle.borderColor}">
                    ${row.qooStatus} · ${row.qooScore}
                </span>
            </strong>
            <small>Traffic-aware QoO impact score</small>
        </div>
        <div class="asn-analysis-kpi">
            <label>Median RTT</label>
            <strong style="color:${colorByRttMs(toNumber(row.medianRtt, 0))}">${formatRttMs(row.medianRtt)}</strong>
            <small>15-minute ASN heatmap median</small>
        </div>
        <div class="asn-analysis-kpi">
            <label>Retransmit</label>
            <strong style="color:${colorByRetransmitPct(toNumber(row.medianRetrans, 0))}">${formatRetransPct(toNumber(row.medianRetrans, 0))}</strong>
            <small>Median retransmit over recent heatmap blocks</small>
        </div>
        <div class="asn-analysis-kpi">
            <label>Recent Flows</label>
            <strong>${formatFlowCount(row.flowCount)}</strong>
            <small>From recently finished two-way flows</small>
        </div>
        <div class="asn-analysis-kpi">
            <label>Recent Traffic</label>
            <strong>${formatVolume(row.totalBytes)}</strong>
            <small>${formatTrafficRateMbps(row.avgMbps)}</small>
        </div>
    `;

    rationale.innerText = rankRationale(row);
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

    const rows = sortedLeaderboardRows();
    state.bubbleChart = ensureChart("asnAnalysisBubbleChart", state.bubbleChart, "bubbleChartTheme");
    if (!state.bubbleChart) {
        return;
    }

    if (!rows.length) {
        state.bubbleChart.clear();
        state.bubbleChart.setOption({
            title: {
                text: "Waiting for executive ASN heatmaps…",
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
            borderColor: row.asn === state.selectedAsn ? (isDarkMode() ? "#ffffff" : "#0f172a") : "rgba(255,255,255,0.28)",
            borderWidth: row.asn === state.selectedAsn ? 2.5 : 1,
            shadowBlur: row.asn === state.selectedAsn ? 18 : 8,
            shadowColor: row.asn === state.selectedAsn ? "rgba(96, 165, 250, 0.35)" : "rgba(15, 23, 42, 0.16)",
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
                        <div class="fw-semibold">${escapeHtml(row.asnName)} (ASN ${row.asn})</div>
                        <div>Traffic: ${formatVolume(row.totalBytes)} (${formatTrafficRateMbps(row.avgMbps)})</div>
                        <div>Median RTT: ${formatRttMs(row.medianRtt)}</div>
                        <div>Median Retransmit: ${formatRetransPct(toNumber(row.medianRetrans, 0))}</div>
                        <div>Recent Flows: ${formatFlowCount(row.flowCount)}</div>
                        <div>QoO: ${row.qooStatus} · ${row.qooScore}</div>
                    </div>
                `;
            },
        },
        xAxis: {
            type: "value",
            name: "Traffic (15m total)",
            nameGap: 28,
            axisLabel: {
                formatter: (value) => formatVolume(value),
            },
        },
        yAxis: {
            type: "value",
            name: "Median RTT",
            nameGap: 18,
            axisLabel: {
                formatter: (value) => value > 0 ? `${Math.round(value)} ms` : "—",
            },
        },
        series: [{
            type: "scatter",
            data: seriesData,
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
    return clamp(Math.sqrt(count) * 4, 14, 54);
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

    const rows = [...visibleAsnRows()].sort((a, b) => {
        const retransmitDiff = toNumber(b.medianRetrans, 0) - toNumber(a.medianRetrans, 0);
        if (retransmitDiff !== 0) return retransmitDiff;
        return b.totalBytes - a.totalBytes;
    });
    if (!rows.length) {
        state.retransmitChart.clear();
        state.retransmitChart.setOption({
            title: {
                text: "Waiting for executive ASN heatmaps…",
                left: "center",
                top: "middle",
                textStyle: { color: NEUTRAL_CHART_COLOR, fontSize: 14, fontWeight: 500 },
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
                        <div class="fw-semibold">${escapeHtml(row.asnName)} (ASN ${row.asn})</div>
                        <div>Retransmit: ${formatRetransPct(toNumber(row.medianRetrans, 0))}</div>
                        <div>Traffic: ${formatVolume(row.totalBytes)} (${formatTrafficRateMbps(row.avgMbps)})</div>
                        <div>Median RTT: ${formatRttMs(row.medianRtt)}</div>
                        <div>QoO: ${row.qooStatus} · ${row.qooScore}</div>
                    </div>
                `;
            },
        },
        xAxis: {
            type: "value",
            axisLabel: {
                formatter: (value) => `${toNumber(value, 0).toFixed(1)}%`,
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
    requestCountryList();
    requestProtocolList();
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

function renderDropdown({ targetId, buttonText, items, emptyText, itemRenderer }) {
    const target = document.getElementById(targetId);
    if (!target) return;
    clearDiv(target);

    const parent = document.createElement("div");
    parent.className = "dropdown";
    const button = document.createElement("button");
    button.className = "btn btn-sm btn-outline-secondary dropdown-toggle";
    button.type = "button";
    button.innerText = buttonText;
    button.setAttribute("data-bs-toggle", "dropdown");
    button.setAttribute("aria-expanded", "false");
    parent.appendChild(button);

    const menu = document.createElement("ul");
    menu.className = "dropdown-menu";
    if (!items.length) {
        const li = document.createElement("li");
        li.className = "dropdown-item disabled";
        li.setAttribute("aria-disabled", "true");
        li.innerText = emptyText;
        menu.appendChild(li);
    } else {
        items.forEach((item) => {
            const rendered = itemRenderer(item);
            if (rendered) {
                menu.appendChild(rendered);
            }
        });
    }
    parent.appendChild(menu);
    target.appendChild(parent);
}

function requestCountryList() {
    listenOnce("CountryList", (msg) => {
        state.countryList = (msg?.data || []).slice().sort((a, b) => toNumber(b?.count, 0) - toNumber(a?.count, 0));
        renderDropdown({
            targetId: "asnAnalysisCountryControl",
            buttonText: "Country",
            items: state.countryList,
            emptyText: "No recent country flow data",
            itemRenderer: (row) => {
                const li = document.createElement("li");
                li.className = "dropdown-item";
                li.innerHTML = `<img alt="${escapeHtml(row.iso_code)}" src="flags/${row.iso_code.toLowerCase()}.svg" width="12" height="12" class="me-2"> ${escapeHtml(row.name)} (${row.count})`;
                li.onclick = () => {
                    state.evidenceScope = { type: "country", value: row.iso_code, label: row.name };
                    loadEvidenceForScope(state.evidenceScope);
                };
                return li;
            },
        });
    });
    wsClient.send({ CountryList: {} });
}

function requestProtocolList() {
    listenOnce("ProtocolList", (msg) => {
        state.protocolList = (msg?.data || []).slice().sort((a, b) => toNumber(b?.count, 0) - toNumber(a?.count, 0));
        renderDropdown({
            targetId: "asnAnalysisProtocolControl",
            buttonText: "Protocol",
            items: state.protocolList,
            emptyText: "No recent protocol flow data",
            itemRenderer: (row) => {
                const li = document.createElement("li");
                li.className = "dropdown-item";
                li.innerText = `${row.protocol} (${row.count})`;
                li.onclick = () => {
                    state.evidenceScope = { type: "protocol", value: row.protocol, label: row.protocol };
                    loadEvidenceForScope(state.evidenceScope);
                };
                return li;
            },
        });
    });
    wsClient.send({ ProtocolList: {} });
}

function loadEvidenceForScope(scope) {
    const token = ++state.activeEvidenceRequestToken;
    state.evidencePage = 0;
    renderFlowContext(`Loading ${scope.type === "asn" ? "selected ASN" : scope.type} flow evidence…`);
    renderFlowRows([]);

    if (!scope || scope.value === null || scope.value === undefined) {
        return;
    }

    if (scope.type === "country") {
        listenOnce("CountryFlowTimeline", (msg) => {
            if (token !== state.activeEvidenceRequestToken) return;
            state.evidenceRows = (msg?.data || []).slice();
            renderFlowEvidence();
        });
        wsClient.send({ CountryFlowTimeline: { iso_code: scope.value } });
        return;
    }

    if (scope.type === "protocol") {
        listenOnce("ProtocolFlowTimeline", (msg) => {
            if (token !== state.activeEvidenceRequestToken) return;
            state.evidenceRows = (msg?.data || []).slice();
            renderFlowEvidence();
        });
        wsClient.send({ ProtocolFlowTimeline: { protocol: scope.value } });
        return;
    }

    listenOnce("AsnFlowTimeline", (msg) => {
        if (token !== state.activeEvidenceRequestToken) return;
        state.evidenceRows = (msg?.data || []).slice();
        renderFlowEvidence();
    });
    wsClient.send({ AsnFlowTimeline: { asn: scope.value } });
}

function evidenceScopeLabel() {
    if (state.evidenceScope.type === "country") {
        return `Showing recent completed flows for country ${state.evidenceScope.label}.`;
    }
    if (state.evidenceScope.type === "protocol") {
        return `Showing recent completed flows for protocol ${state.evidenceScope.label}.`;
    }
    const selectedLabel = state.selectedAsn ? asnDisplayName(state.selectedAsn) : "the selected ASN";
    return `Showing recent completed flows for ${selectedLabel}.`;
}

function renderFlowContext(message = null) {
    const target = document.getElementById("asnAnalysisFlowContext");
    if (!target) return;
    target.innerText = message || evidenceScopeLabel();
}

function renderFlowEvidence() {
    const rows = [...state.evidenceRows];
    rows.sort(SORT_OPTIONS[state.evidenceSort] || SORT_OPTIONS.bytes);
    state.evidenceRows = rows;
    renderFlowContext();
    renderFlowRows(rows);
}

function renderFlowRows(allRows) {
    const tbody = document.getElementById("asnAnalysisFlowTableBody");
    const paginator = document.getElementById("asnAnalysisPaginator");
    const prev = document.getElementById("asnAnalysisPrevPage");
    const next = document.getElementById("asnAnalysisNextPage");
    if (!tbody || !paginator || !prev || !next) {
        return;
    }

    clearDiv(tbody);
    const pageCount = Math.max(1, Math.ceil(allRows.length / EVIDENCE_ITEMS_PER_PAGE));
    state.evidencePage = clamp(state.evidencePage, 0, pageCount - 1);
    paginator.innerText = `Page ${state.evidencePage + 1} of ${pageCount}`;
    prev.disabled = state.evidencePage === 0;
    next.disabled = state.evidencePage >= pageCount - 1;

    if (!allRows.length) {
        tbody.innerHTML = "<tr><td colspan='6' class='asn-analysis-empty'>No recent flows match this evidence scope yet.</td></tr>";
        state.evidenceMinTime = Number.MAX_SAFE_INTEGER;
        state.evidenceMaxTime = Number.MIN_SAFE_INTEGER;
        return;
    }

    const start = state.evidencePage * EVIDENCE_ITEMS_PER_PAGE;
    const pageRows = allRows.slice(start, start + EVIDENCE_ITEMS_PER_PAGE);

    state.evidenceMinTime = Math.min(...allRows.map((row) => toNumber(row.start, 0)));
    state.evidenceMaxTime = Math.max(...allRows.map((row) => toNumber(row.end, 0)));

    pageRows.forEach((row, index) => {
        const tr = document.createElement("tr");
        const remoteIp = String(row.remote_ip || "").trim();
        const timelineId = `asnAnalysisFlowCanvas${state.evidencePage}_${index}`;
        const bytesDown = scaleNumber(toNumber(row?.total_bytes?.down, 0), 0);
        const bytesUp = scaleNumber(toNumber(row?.total_bytes?.up, 0), 0);
        const rttDown = row?.rtt?.[0] ? scaleNanos(row.rtt[0].nanoseconds, 0) : "-";
        const rttUp = row?.rtt?.[1] ? scaleNanos(row.rtt[1].nanoseconds, 0) : "-";
        const circuitTitle = row.circuit_name || row.circuit_id || "Unmapped circuit";
        const circuitHtml = row.circuit_id
            ? `<a class="redactable" href="/circuit.html?id=${encodeURIComponent(row.circuit_id)}">${escapeHtml(circuitTitle)}</a>`
            : `<span class="redactable">${escapeHtml(circuitTitle)}</span>`;
        const startLabel = toNumber(row.start, 0) > 0 ? new Date(toNumber(row.start, 0) * 1000).toLocaleTimeString() : "Unknown";
        const durationLabel = toNumber(row.duration_nanos, 0) > 0 ? scaleNanos(toNumber(row.duration_nanos, 0), 0) : "—";

        tr.innerHTML = `
            <td>
                <div class="asn-analysis-flow-circuit">
                    <strong>${circuitHtml}</strong>
                    <span>${escapeHtml(row.circuit_id || "No circuit id")}</span>
                </div>
            </td>
            <td class="redactable">
                <div class="asn-analysis-flow-meta">
                    <strong>${escapeHtml(remoteIp || "—")}</strong>
                    <span>Started ${escapeHtml(startLabel)}</span>
                </div>
            </td>
            <td>
                <div class="asn-analysis-flow-meta">
                    <strong>${escapeHtml(row.protocol || "—")}</strong>
                    <span>Duration ${escapeHtml(durationLabel)}</span>
                </div>
            </td>
            <td>
                <div class="asn-analysis-flow-meta">
                    <strong>${bytesDown} / ${bytesUp}</strong>
                    <span>Down / Up bytes</span>
                </div>
            </td>
            <td>
                <div class="asn-analysis-flow-meta">
                    <strong>${rttDown} / ${rttUp}</strong>
                    <span>Down / Up RTT</span>
                </div>
            </td>
            <td><canvas id="${timelineId}" data-flow-index="${start + index}"></canvas></td>
        `;

        const remoteCell = tr.children[1];
        if (remoteIp) {
            const button = document.createElement("button");
            button.type = "button";
            button.className = "btn btn-link btn-sm p-0 ms-2";
            button.title = "Exclude RTT for this remote endpoint";
            button.innerHTML = "<i class='fa fa-ban'></i>";
            button.addEventListener("click", (event) => {
                event.preventDefault();
                event.stopPropagation();
                openFlowRttExcludeWizard({ remoteIp, sourceLabel: "ASN Analysis" });
            });
            remoteCell.appendChild(button);
        }

        tbody.appendChild(tr);
    });

    requestAnimationFrame(() => {
        window.setTimeout(drawEvidenceTimelines, 0);
    });
}

function timeToCanvasX(time, width) {
    const range = state.evidenceMaxTime - state.evidenceMinTime;
    if (!Number.isFinite(range) || range <= 0) {
        return 0;
    }
    return ((time - state.evidenceMinTime) / range) * width;
}

function drawEvidenceTimelines() {
    const canvasNodes = document.querySelectorAll("#asnAnalysisFlowTableBody canvas");
    if (!canvasNodes.length) {
        return;
    }
    const style = getComputedStyle(document.body);
    const regionBg = style.getPropertyValue("--bs-tertiary-bg") || "rgba(148,163,184,0.18)";
    const axisColor = style.getPropertyValue("--bs-secondary-color") || "#94a3b8";
    const lineColor = style.getPropertyValue("--bs-primary") || "#60a5fa";

    canvasNodes.forEach((canvas) => {
        const flowIndex = toNumber(canvas.dataset.flowIndex, -1);
        const row = state.evidenceRows[flowIndex];
        if (!row) return;

        const { width, height } = canvas.getBoundingClientRect();
        canvas.width = Math.max(1, Math.round(width));
        canvas.height = Math.max(1, Math.round(height));
        const ctx = canvas.getContext("2d");
        if (!ctx) return;

        const startX = timeToCanvasX(toNumber(row.start, 0), canvas.width);
        const endX = timeToCanvasX(toNumber(row.end, 0), canvas.width);
        const spanWidth = Math.max(2, endX - startX);

        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.fillStyle = regionBg.trim() || "rgba(148,163,184,0.18)";
        ctx.fillRect(startX, 0, spanWidth, canvas.height);

        ctx.strokeStyle = axisColor.trim() || "#94a3b8";
        ctx.beginPath();
        ctx.moveTo(startX, canvas.height / 2);
        ctx.lineTo(endX, canvas.height / 2);
        ctx.stroke();

        ctx.strokeStyle = "#ef4444";
        (row.retransmit_times_down || []).forEach((time) => {
            const x = timeToCanvasX(toNumber(time, 0), canvas.width);
            ctx.beginPath();
            ctx.moveTo(x, canvas.height / 2);
            ctx.lineTo(x, canvas.height);
            ctx.stroke();
        });
        (row.retransmit_times_up || []).forEach((time) => {
            const x = timeToCanvasX(toNumber(time, 0), canvas.width);
            ctx.beginPath();
            ctx.moveTo(x, 0);
            ctx.lineTo(x, canvas.height / 2);
            ctx.stroke();
        });

        const samples = row.throughput || [];
        if (!samples.length) return;

        const maxDown = Math.max(1, ...samples.map((entry) => toNumber(entry.down, 0)));
        const maxUp = Math.max(1, ...samples.map((entry) => toNumber(entry.up, 0)));
        const sampleWidth = spanWidth / Math.max(1, samples.length);

        ctx.strokeStyle = lineColor.trim() || "#60a5fa";
        ctx.beginPath();
        let x = startX;
        ctx.moveTo(x, canvas.height / 2);
        samples.forEach((entry) => {
            const y = (canvas.height / 2) - ((toNumber(entry.down, 0) / maxDown) * ((canvas.height - 4) / 2));
            ctx.lineTo(x, y);
            x += sampleWidth;
        });
        ctx.stroke();

        ctx.beginPath();
        x = startX;
        ctx.moveTo(x, canvas.height / 2);
        samples.forEach((entry) => {
            const y = (canvas.height / 2) + ((toNumber(entry.up, 0) / maxUp) * ((canvas.height - 4) / 2));
            ctx.lineTo(x, y);
            x += sampleWidth;
        });
        ctx.stroke();
    });
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

function wireControls() {
    const refreshButton = document.getElementById("asnAnalysisRefreshButton");
    const evidenceAsnButton = document.getElementById("asnAnalysisEvidenceSelectedAsn");
    const flowSort = document.getElementById("asnAnalysisFlowSort");
    const prev = document.getElementById("asnAnalysisPrevPage");
    const next = document.getElementById("asnAnalysisNextPage");

    refreshButton?.addEventListener("click", () => {
        requestLists();
        state.executivePollHandles.forEach((handle) => handle?.refresh?.());
        const top = sortedLeaderboardRows()[0];
        if (top) {
            setSelectedAsn(top.asn);
        }
        renderAll();
    });

    evidenceAsnButton?.addEventListener("click", () => {
        if (state.selectedAsn === null) return;
        state.evidenceScope = { type: "asn", value: state.selectedAsn, label: asnDisplayName(state.selectedAsn) };
        loadEvidenceForScope(state.evidenceScope);
    });

    flowSort?.addEventListener("change", () => {
        state.evidenceSort = flowSort.value;
        renderFlowEvidence();
    });

    prev?.addEventListener("click", () => {
        state.evidencePage = Math.max(0, state.evidencePage - 1);
        renderFlowRows(state.evidenceRows);
    });

    next?.addEventListener("click", () => {
        state.evidencePage += 1;
        renderFlowRows(state.evidenceRows);
    });
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
