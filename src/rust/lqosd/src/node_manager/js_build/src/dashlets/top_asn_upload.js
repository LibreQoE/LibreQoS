import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {colorByRetransmitPct} from "../helpers/color_scales";

class TopAsnSankeyGraphUp extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            tooltip: { show: true },
            series: [
                {
                    type: 'sankey',
                    data: [],
                    links: [],
                    nodeAlign: 'right',
                    lineStyle: { color: 'gradient', curveness: 0.5 },
                }
            ]
        };
        this.chart.setOption(this.option);
    }
    update(items) {
        const nodes = [{ name: 'Shaper' }];
        const links = [];
        items.forEach(row => {
            const color = colorByRetransmitPct(row.rxmit ?? 0);
            nodes.push({ name: row.name, itemStyle: { color } });
            links.push({ source: 'Shaper', target: row.name, value: row.value });
        });
        this.option.series[0].data = nodes;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

export class ShaperTopAsnUpload extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.last = null;
        this._emptyId = this.id + "_empty";
        // Smoothing state
        this._smooth = new Map(); // name -> {v, rx, linger}
        this._ALPHA = 0.3;
        this._ALPHA_RX = 0.2;
        this._DECAY = 0.15;
        this._LINGER = 3;
    }
    canBeSlowedDown() { return true; }
    title() { return "Shaper Top ASN (Upload)"; }
    tooltip() { return "<h5>Top Upload ASNs</h5><p>Aggregated from live top flows by rate (bps)."; }
    subscribeTo() { return ["AsnTopUpload", "TopFlowsRate"]; }
    buildContainer() { let b = super.buildContainer(); b.appendChild(this.graphDiv()); return b; }
    setup() { this.graph = new TopAsnSankeyGraphUp(this.graphDivId()); if (this.last) this.graph.update(this.last); }
    _showEmpty(show, msg = "No recent data") {
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
    _smoothItems(items) {
        const present = new Set();
        items.forEach(it => {
            present.add(it.name);
            const prev = this._smooth.get(it.name) || { v: 0, rx: 0, linger: this._LINGER };
            const v = (1 - this._ALPHA) * prev.v + this._ALPHA * it.value;
            const rx = (1 - this._ALPHA_RX) * prev.rx + this._ALPHA_RX * (it.rxmit || 0);
            this._smooth.set(it.name, { v, rx, linger: this._LINGER });
        });
        for (const [name, state] of Array.from(this._smooth.entries())) {
            if (!present.has(name)) {
                const v = state.v * (1 - this._DECAY);
                const linger = state.linger - 1;
                if (linger <= 0 || v < 1) {
                    this._smooth.delete(name);
                } else {
                    this._smooth.set(name, { v, rx: state.rx, linger });
                }
            }
        }
        let out = Array.from(this._smooth.entries()).map(([name, s]) => ({ name, value: s.v, rxmit: s.rx }));
        out.sort((a,b)=> b.value - a.value);
        return out.slice(0, 9);
    }
    onMessage(msg) {
        if (msg.event === "AsnTopUpload") {
            let items = (msg.data || []).map(r => ({ name: r.name, value: Number(r.value||0), rxmit: Number(r.retransmit_percent||0) }));
            items = this._smoothItems(items);
            const hasData = items.some(it => it.value > 0.5);
            this._showEmpty(!hasData);
            if (hasData) {
                this.last = items;
                this.graph.update(items);
            }
            return;
        }
        if (msg.event !== "TopFlowsRate") return;
        const map = new Map();
        (msg.data || []).forEach(f => {
            const key = f.remote_asn_name && f.remote_asn_name.length > 0 ? f.remote_asn_name : (f.remote_ip || "Unknown");
            const val = Number(f.rate_estimate_bps?.up || 0);
            const rxmitU = (f.tcp_retransmits?.up || 0) / Math.max(1, f.packets_sent?.up || 0) * 100.0; // %
            const cur = map.get(key) || { name: key, value: 0, rxmit: 0, samples: 0 };
            cur.value += val;
            cur.rxmit += isFinite(rxmitU) ? rxmitU : 0;
            cur.samples += 1;
            map.set(key, cur);
        });
        let items = Array.from(map.values()).map(v => ({ name: v.name, value: v.value, rxmit: v.samples ? v.rxmit / v.samples : 0 }));
        items.sort((a,b)=> b.value - a.value);
        items = items.slice(0, 9);
        items = this._smoothItems(items);
        const hasData = items.some(it => it.value > 0.5);
        this._showEmpty(!hasData);
        if (hasData) {
            this.last = items;
            this.graph.update(items);
        }
    }
}
