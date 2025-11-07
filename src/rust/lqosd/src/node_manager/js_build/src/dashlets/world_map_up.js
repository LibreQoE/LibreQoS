import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {WorldMap3DGraph as _WorldMap3DGraph} from "./world_map_down";
import {colorByRttMs} from "../helpers/color_scales";

function ensureWorldMap() {
    try {
        if (typeof echarts !== 'undefined' && echarts.getMap && echarts.getMap('world')) {
            return Promise.resolve();
        }
    } catch (_) {}
    if (window._worldMapPromise) return window._worldMapPromise;
    window._worldMapPromise = new Promise((resolve, reject) => {
        const load = (src, onfail) => {
            const s = document.createElement('script');
            s.src = src;
            s.onload = () => resolve();
            s.onerror = () => onfail ? onfail() : reject();
            document.head.appendChild(s);
        };
        // Prefer local vendor file if shipped (static2/vendor/world.js)
        load('vendor/world.js', () => {
            load('https://fastly.jsdelivr.net/npm/echarts@4.9.0/map/js/world.js');
        });
    });
    return window._worldMapPromise;
}

// Reuse the same graph implementation; differ in title/tooltip only
class WorldMap3DGraph extends _WorldMap3DGraph {}

export class ShaperWorldMapUp extends BaseDashlet {
    constructor(slot){ super(slot); this.last = null; this._emptyId = this.id + "_empty"; }
    canBeSlowedDown(){ return true; }
    title(){ return "Shaper World Map (Upload)"; }
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
        const minSize = 1, maxSize = 8;
        const out = [];
        for (let i=0;i<rows.length;i++) {
            const lat = rows[i][0], lon = rows[i][1];
            const bytes = Number(rows[i][3] || 0);
            const rtt = Math.min(200, Number(rows[i][4]||0));
            let norm = 0;
            if (maxBytes > 0) {
                norm = Math.sqrt(bytes / maxBytes);
                if (!isFinite(norm) || norm < 0) norm = 0;
                if (norm > 1) norm = 1;
            }
            const size = Math.round(minSize + (maxSize - minSize) * norm);
            const color = colorByRttMs(rtt, 200);
            out.push({ value: [lon, lat], symbolSize: size, itemStyle: { color, opacity: 0.6 } });
        }
        const hasData = out.length > 0;
        this._showEmpty(!hasData);
        if (hasData) {
            this.last = out;
            ensureWorldMap().then(() => {
                this.graph.update(out);
            }).catch(() => {
                this.graph.update(out);
            });
        }
    }
}
