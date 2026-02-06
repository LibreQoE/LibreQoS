import { clearDiv } from "./helpers/builders";
import { isDarkMode } from "./helpers/dark_mode";
import { DashboardGraph } from "./graphs/dashboard_graph";
import { get_ws_client } from "./pubsub/ws";

const wsClient = get_ws_client();

let treeGraph = null;
let lastRoot = null;

const DEPTH_STORAGE_KEY = "cpu_tree_depth_levels";
const DEFAULT_DEPTH_LEVELS = 3;
let depthLevels = DEFAULT_DEPTH_LEVELS;

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function setStatus(html, isError = false) {
    const status = document.getElementById("cpuTreeStatus");
    if (!status) return;
    status.className = isError ? "text-danger small mb-2" : "text-muted small mb-2";
    status.innerHTML = html;
}

function requestCpuSiteTree() {
    return new Promise((resolve) => {
        const timeout = setTimeout(() => resolve(null), 10000);
        listenOnce("CpuAffinitySiteTree", (msg) => {
            clearTimeout(timeout);
            resolve(msg && msg.data ? msg.data : null);
        });
        wsClient.send({ CpuAffinitySiteTree: {} });
    });
}

function toInitialTreeDepth(levels) {
    if (levels === -1) return -1;
    const n = Number(levels);
    if (!Number.isFinite(n) || n < 1) return DEFAULT_DEPTH_LEVELS - 1;
    // ECharts uses 0-based depth (root=0). Our UI uses "levels" (root=1).
    return Math.max(0, Math.floor(n) - 1);
}

function allowedDepthLevelsFromSelect(selectEl) {
    const allowed = new Set();
    if (!selectEl || !selectEl.options) return allowed;
    for (const opt of Array.from(selectEl.options)) {
        const n = parseInt(opt.value, 10);
        if (Number.isFinite(n)) {
            allowed.add(n);
        }
    }
    return allowed;
}

function normalizeDepthLevels(levels, allowed) {
    let n = Number(levels);
    if (!Number.isFinite(n)) {
        n = DEFAULT_DEPTH_LEVELS;
    }
    n = Math.trunc(n);

    // Migration: removed depth level 2 (CPU-only view).
    if (n === 2) {
        n = DEFAULT_DEPTH_LEVELS;
    }

    if (n !== -1 && n < 1) {
        n = DEFAULT_DEPTH_LEVELS;
    }

    if (allowed && allowed.size > 0 && !allowed.has(n)) {
        if (allowed.has(DEFAULT_DEPTH_LEVELS)) {
            n = DEFAULT_DEPTH_LEVELS;
        } else {
            const sorted = Array.from(allowed).sort((a, b) => a - b);
            n = sorted.length > 0 ? sorted[0] : DEFAULT_DEPTH_LEVELS;
        }
    }

    return n;
}

function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, (c) => {
        switch (c) {
            case "&":
                return "&amp;";
            case "<":
                return "&lt;";
            case ">":
                return "&gt;";
            case '"':
                return "&quot;";
            case "'":
                return "&#39;";
            default:
                return c;
        }
    });
}

function buildTreeOption(root, levels) {
    const dark = isDarkMode();
    const accent = dark ? "#4992ff" : "#61a0a8";
    const lineColor = dark ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.18)";
    const lineEmphasisColor = dark ? "rgba(255,255,255,0.42)" : "rgba(0,0,0,0.32)";
    const labelColor = dark ? "rgba(235, 240, 250, 0.95)" : "#1f2937";
    const labelBg = dark ? "rgba(255,255,255,0.06)" : "rgba(255,255,255,0.85)";
    const labelBorder = dark ? "rgba(255,255,255,0.12)" : "rgba(0,0,0,0.06)";
    const nodeFill = dark ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.06)";
    const nodeBorder = dark ? "rgba(255,255,255,0.35)" : "rgba(0,0,0,0.18)";

    return {
        tooltip: {
            trigger: "item",
            triggerOn: "mousemove",
            confine: true,
            borderWidth: 1,
            borderColor: labelBorder,
            backgroundColor: dark ? "rgba(15, 15, 20, 0.92)" : "rgba(255,255,255,0.98)",
            textStyle: {
                color: labelColor,
                fontSize: 12,
            },
            formatter: (params) => {
                const data = params && params.data ? params.data : null;
                const name = escapeHtml((data && data.name) || (params && params.name) || "");
                const childCount =
                    data && Array.isArray(data.children) ? data.children.length : 0;
                const childLine =
                    childCount > 0
                        ? `<div style="opacity:0.75; margin-top:2px;">Children: ${childCount}</div>`
                        : "";
                return `<div style="font-size:12px;"><div style="font-weight:600;">${name}</div>${childLine}</div>`;
            },
        },
        series: [
            {
                type: "tree",
                data: [root],
                top: "2%",
                left: "2%",
                bottom: "2%",
                right: "20%",
                roam: true,
                symbolSize: 9,
                edgeShape: "polyline",
                expandAndCollapse: true,
                initialTreeDepth: toInitialTreeDepth(levels),
                itemStyle: {
                    color: nodeFill,
                    borderColor: nodeBorder,
                    borderWidth: 1,
                },
                lineStyle: {
                    color: lineColor,
                    width: 1.15,
                    opacity: 0.75,
                    curveness: 0.35,
                },
                label: {
                    position: "left",
                    verticalAlign: "middle",
                    align: "right",
                    fontSize: 12,
                    color: labelColor,
                    backgroundColor: labelBg,
                    borderColor: labelBorder,
                    borderWidth: 1,
                    borderRadius: 4,
                    padding: [3, 6],
                },
                leaves: {
                    label: {
                        position: "right",
                        verticalAlign: "middle",
                        align: "left",
                        fontSize: 12,
                        color: labelColor,
                        backgroundColor: labelBg,
                        borderColor: labelBorder,
                        borderWidth: 1,
                        borderRadius: 4,
                        padding: [3, 6],
                    },
                },
                emphasis: {
                    focus: "descendant",
                    itemStyle: {
                        borderColor: accent,
                        borderWidth: 1.5,
                        shadowBlur: 10,
                        shadowColor: accent,
                    },
                    lineStyle: {
                        color: lineEmphasisColor,
                        width: 2,
                        opacity: 1,
                    },
                    label: {
                        borderColor: accent,
                    },
                },
                animationDuration: 550,
                animationDurationUpdate: 750,
            },
        ],
    };
}

function renderTree(root) {
    if (!root) {
        setStatus("No tree data found. Run LibreQoS to generate queuingStructure.json.", true);
        return;
    }
    if (!treeGraph) {
        treeGraph = new DashboardGraph("cpuTreeChart");
    }
    treeGraph.option = buildTreeOption(root, depthLevels);
    try {
        treeGraph.chart.setOption(treeGraph.option, true);
        treeGraph.chart.hideLoading();
    } catch (e) {
        setStatus("Failed to render chart (echarts unavailable).", true);
    }
}

async function refresh() {
    setStatus('<i class="fa fa-spinner fa-spin"></i> Loading...');
    const root = await requestCpuSiteTree();
    if (!root || !root.children || root.children.length === 0) {
        setStatus("No site placement data found. Run LibreQoS to generate queuingStructure.json.", true);
        const chartDom = document.getElementById("cpuTreeChart");
        if (chartDom) clearDiv(chartDom);
        return;
    }
    lastRoot = root;
    setStatus("Pan/zoom with mouse. Click nodes to expand/collapse.");
    renderTree(root);
}

function loadDepthSetting() {
    try {
        const stored = window.localStorage ? window.localStorage.getItem(DEPTH_STORAGE_KEY) : null;
        const n = stored !== null ? parseInt(stored, 10) : DEFAULT_DEPTH_LEVELS;
        if (Number.isFinite(n)) {
            depthLevels = n;
        }
    } catch (_) {}
}

function saveDepthSetting() {
    try {
        if (window.localStorage) {
            window.localStorage.setItem(DEPTH_STORAGE_KEY, String(depthLevels));
        }
    } catch (_) {}
}

// Wire up controls
const btnRefresh = document.getElementById("btnRefresh");
if (btnRefresh) {
    btnRefresh.onclick = () => refresh();
}

loadDepthSetting();

const depthSelect = document.getElementById("treeDepth");
if (depthSelect) {
    const allowed = allowedDepthLevelsFromSelect(depthSelect);
    const normalized = normalizeDepthLevels(depthLevels, allowed);
    if (normalized !== depthLevels) {
        depthLevels = normalized;
        saveDepthSetting();
    }
    depthSelect.value = String(depthLevels);
    depthSelect.onchange = () => {
        const n = parseInt(depthSelect.value, 10);
        if (Number.isFinite(n)) {
            depthLevels = n;
            saveDepthSetting();
        }
        if (lastRoot) {
            renderTree(lastRoot);
        }
    };
} else {
    const normalized = normalizeDepthLevels(depthLevels, null);
    if (normalized !== depthLevels) {
        depthLevels = normalized;
        saveDepthSetting();
    }
}

const btnReset = document.getElementById("btnReset");
if (btnReset) {
    btnReset.onclick = () => {
        depthLevels = DEFAULT_DEPTH_LEVELS;
        try {
            if (window.localStorage) {
                window.localStorage.removeItem(DEPTH_STORAGE_KEY);
            }
        } catch (_) {}
        if (depthSelect) {
            depthSelect.value = String(depthLevels);
        }
        if (treeGraph && treeGraph.chart) {
            try {
                treeGraph.chart.dispatchAction({ type: "restore" });
            } catch (_) {}
        }
        if (lastRoot) {
            renderTree(lastRoot);
        }
    };
}

window.addEventListener("resize", () => {
    if (treeGraph && treeGraph.chart) {
        try {
            treeGraph.chart.resize();
        } catch (_) {}
    }
});

refresh();
