import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {colorByRttMs} from "../helpers/color_scales";
import {isDarkMode} from "../helpers/dark_mode";

function ensureWorldMap() {
    try {
        if (typeof echarts !== 'undefined' && echarts.getMap && echarts.getMap('world')) {
            return Promise.resolve();
        }
    } catch (_) {}
    if (window._worldMapPromise) return window._worldMapPromise;
    window._worldMapPromise = new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.id = 'echarts_world_js';
        s.src = 'https://fastly.jsdelivr.net/npm/echarts@4.9.0/map/js/world.js';
        s.onload = () => resolve();
        s.onerror = () => reject();
        document.head.appendChild(s);
    });
    return window._worldMapPromise;
}

export class WorldMap3DGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        const dark = isDarkMode();
        this.option = {
            geo3D: {
                map: 'world',
                shading: 'realistic',
                silent: true,
                environment: dark ? '#000' : '#eee',
                realisticMaterial: { roughness: 0.8, metalness: 0 },
                postEffect: { enable: true },
                groundPlane: { show: false },
                light: { main: { intensity: 1, alpha: 30 }, ambient: { intensity: 0 } },
                viewControl: { distance: 70, alpha: 89, panMouseButton: 'left', rotateMouseButton: 'right' },
                itemStyle: { color: dark ? '#000' : '#bcbcbc' },
                regionHeight: 0.5
            },
            series: [
                {
                    type: 'scatter3D', coordinateSystem: 'geo3D', blendMode: 'lighter',
                    symbolSize: 2, lineStyle: { width: 0.2, opacity: 0.05 }, data: []
                }
            ]
        };
        this.chart.setOption(this.option);
    }
    update(data){
        this.chart.hideLoading();
        this.option.series[0].data = data;
        this.chart.setOption(this.option);
    }
    onThemeChange(){
        const dark = isDarkMode();
        if (!this.option.geo3D) this.option.geo3D = {};
        if (!this.option.geo3D.itemStyle) this.option.geo3D.itemStyle = {};
        this.option.geo3D.environment = dark ? '#000' : '#eee';
        this.option.geo3D.itemStyle.color = dark ? '#000' : '#bcbcbc';
        this.chart.setOption(this.option, true);
    }
}

export class ShaperWorldMapDown extends BaseDashlet {
    constructor(slot){ super(slot); this.last = null; this._emptyId = this.id + "_empty"; }
    canBeSlowedDown(){ return true; }
    title(){ return "Shaper World Map (Download)"; }
    tooltip(){ return "<h5>World Map</h5><p>Endpoint locations sized by traffic and colored by RTT."; }
    subscribeTo(){ return ["EndpointLatLon"]; }
    buildContainer(){ let b=super.buildContainer(); b.appendChild(this.graphDiv()); return b; }
    setup(){ this.graph = new WorldMap3DGraph(this.graphDivId()); if (this.last) this.graph.update(this.last); }
    _showEmpty(show, msg = "No recent data"){
        const card = document.getElementById(this.id);
        if (!card) return;
        let empty = document.getElementById(this._emptyId);
        if (!empty) {
            empty = document.createElement('div');
            empty.id = this._emptyId;
            empty.classList.add('text-center','text-muted','small');
            empty.style.padding = '12px';
            card.appendChild(empty);
        }
        empty.textContent = msg;
        const graph = document.getElementById(this.graphDivId());
        if (show) {
            empty.style.display = '';
            if (graph) graph.style.display = 'none';
        } else {
            empty.style.display = 'none';
            if (graph) graph.style.display = '';
        }
    }
    onMessage(msg){
        if (msg.event !== "EndpointLatLon") return;
        const rows = msg.data || [];
        let maxBytes = 0;
        for (let i=0;i<rows.length;i++) {
            const b = Number(rows[i][3] || 0);
            if (b > maxBytes) maxBytes = b;
        }
        const minSize = 2, maxSize = 14;
        const out = [];
        for (let i=0;i<rows.length;i++) {
            const lat = rows[i][0], lon = rows[i][1];
            const bytes = Number(rows[i][3] || 0);
            const rtt = Math.min(200, Number(rows[i][4]||0));
            const color = colorByRttMs(rtt, 200);
            let norm = 0;
            if (maxBytes > 0) {
                norm = Math.sqrt(bytes / maxBytes);
                if (!isFinite(norm) || norm < 0) norm = 0;
                if (norm > 1) norm = 1;
            }
            const size = Math.round(minSize + (maxSize - minSize) * norm);
            out.push({ value: [lon, lat], symbolSize: size, itemStyle: { color } });
        }
        const hasData = out.length > 0;
        this._showEmpty(!hasData);
        if (hasData) {
            this.last = out;
            ensureWorldMap().then(() => {
                this.graph.update(out);
            }).catch(() => {
                // If the world map fails to load, still attempt to render points
                this.graph.update(out);
            });
        }
    }
}
