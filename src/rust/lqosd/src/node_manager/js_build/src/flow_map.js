import { DashboardGraph } from "./graphs/dashboard_graph";
import {colorByRttMs} from "./helpers/color_scales";
import {toNumber} from "./lq_js_common/helpers/scaling";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();

const POLL_MS = 1000;
const RESPONSE_TIMEOUT_MS = 2000;
const MIN_POINTS = 3;
const MIN_TOTAL_BYTES = 1_000_000;

function listenOnceWithTimeout(eventName, timeoutMs, handler, onTimeout) {
    let done = false;
    const wrapped = (msg) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    const timer = setTimeout(() => {
        if (done) return;
        done = true;
        wsClient.off(eventName, wrapped);
        onTimeout();
    }, timeoutMs);
    wsClient.on(eventName, wrapped);
    return { cancel: () => { wsClient.off(eventName, wrapped); clearTimeout(timer); } };
}

function makeOverlay(container, id) {
    container.style.position = "relative";

    const overlay = document.createElement("div");
    overlay.id = id;
    overlay.style.position = "absolute";
    overlay.style.inset = "0";
    overlay.style.display = "none";
    overlay.style.alignItems = "center";
    overlay.style.justifyContent = "center";
    overlay.style.pointerEvents = "none";
    overlay.style.zIndex = "10";
    overlay.style.padding = "16px";

    const panel = document.createElement("div");
    panel.style.background = "var(--lqos-surface)";
    panel.style.border = "1px solid var(--lqos-border)";
    panel.style.borderRadius = "var(--lqos-radius-lg)";
    panel.style.boxShadow = "var(--lqos-shadow-sm)";
    panel.style.padding = "14px 18px";
    panel.style.maxWidth = "560px";
    panel.style.textAlign = "center";
    panel.style.backdropFilter = "blur(10px)";
    panel.style.webkitBackdropFilter = "blur(10px)";

    const title = document.createElement("div");
    title.style.fontWeight = "700";
    title.style.fontSize = "1.1rem";

    const subtitle = document.createElement("div");
    subtitle.className = "text-muted";
    subtitle.style.marginTop = "4px";

    panel.appendChild(title);
    panel.appendChild(subtitle);
    overlay.appendChild(panel);
    container.appendChild(overlay);

    return {
        show: (t, s) => {
            title.textContent = t;
            subtitle.textContent = s || "";
            overlay.style.display = "flex";
        },
        hide: () => {
            overlay.style.display = "none";
        },
    };
}

class FlowMap extends DashboardGraph {
    constructor(id) {
        super(id);
        let data = [];
        this.option = {
            geo3D: {
                map: 'world',
                shading: 'realistic',
                silent: true,
                environment: '#333',
                realisticMaterial: {
                    roughness: 0.8,
                    metalness: 0
                },
                postEffect: {
                    enable: true
                },
                groundPlane: {
                    show: false
                },
                light: {
                    main: {
                        intensity: 1,
                        alpha: 30
                    },
                    ambient: {
                        intensity: 0
                    }
                },
                viewControl: {
                    distance: 70,
                    alpha: 89,
                    panMouseButton: 'left',
                    rotateMouseButton: 'right'
                },
                itemStyle: {
                    color: '#000'
                },
                regionHeight: 0.5
            },
            series: [
                {
                    type: 'scatter3D',
                    coordinateSystem: 'geo3D',
                    blendMode: 'lighter',
                    lineStyle: {
                        width: 0.2,
                        opacity: 0.05
                    },
                    symbolSize: 2,
                    data: data
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();
        this.option.series[0].data = data;
        this.chart.setOption(this.option);
    }
}

const map = new FlowMap("flowMap");
const overlay = makeOverlay(map.dom, "flowMapOverlay");

function updateMap() {
    listenOnceWithTimeout("FlowMap", RESPONSE_TIMEOUT_MS, (msg) => {
        const data = msg && msg.data ? msg.data : [];
        const totalBytes = data.reduce((acc, d) => acc + toNumber(d?.[3], 0), 0);

        if (data.length < MIN_POINTS || totalBytes < MIN_TOTAL_BYTES) {
            overlay.show("Insufficient data", "Not enough recent flow traffic to render the map yet.");
            map.update([]);
        } else {
            overlay.hide();
            const output = data.map((d) => {
                const rttMs = toNumber(d?.[4], 0) / 1_000_000;
                const color = colorByRttMs(rttMs);
                return {
                    value: [toNumber(d?.[1], 0), toNumber(d?.[0], 0)], // It wants lon/lat
                    itemStyle: { color },
                };
            });
            map.update(output);
        }

        setTimeout(updateMap, POLL_MS);
    }, () => {
        overlay.show("Waiting for data", "No FlowMap websocket response received yet.");
        map.update([]);
        setTimeout(updateMap, POLL_MS);
    });
    wsClient.send({ FlowMap: {} });
}

window.addEventListener("resize", () => {
    try {
        map.chart.resize();
    } catch (e) {
        // ignore
    }
});

overlay.show("Waiting for data", "Requesting recent flow endpoints...");
updateMap();
