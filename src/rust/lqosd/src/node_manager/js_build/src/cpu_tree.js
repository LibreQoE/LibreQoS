import { clearDiv } from "./helpers/builders";
import { DashboardGraph } from "./graphs/dashboard_graph";
import { get_ws_client } from "./pubsub/ws";

const wsClient = get_ws_client();

let treeGraph = null;

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

function buildTreeOption(root) {
    return {
        tooltip: {
            trigger: "item",
            triggerOn: "mousemove",
        },
        series: [
            {
                type: "tree",
                data: [root],
                top: "2%",
                left: "2%",
                bottom: "2%",
                right: "18%",
                roam: true,
                symbolSize: 10,
                edgeShape: "polyline",
                expandAndCollapse: true,
                initialTreeDepth: -1,
                lineStyle: {
                    width: 1.25,
                    curveness: 0.5,
                },
                label: {
                    position: "left",
                    verticalAlign: "middle",
                    align: "right",
                    fontSize: 12,
                },
                leaves: {
                    label: {
                        position: "right",
                        verticalAlign: "middle",
                        align: "left",
                        fontSize: 12,
                    },
                },
                emphasis: {
                    focus: "descendant",
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
    treeGraph.option = buildTreeOption(root);
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
    setStatus("Pan/zoom with mouse. Click nodes to expand/collapse.");
    renderTree(root);
}

// Wire up controls
const btnRefresh = document.getElementById("btnRefresh");
if (btnRefresh) {
    btnRefresh.onclick = () => refresh();
}

window.addEventListener("resize", () => {
    if (treeGraph && treeGraph.chart) {
        try {
            treeGraph.chart.resize();
        } catch (_) {}
    }
});

refresh();
