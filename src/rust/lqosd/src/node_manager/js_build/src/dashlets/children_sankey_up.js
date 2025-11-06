import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {colorByRetransmitPct} from "../helpers/color_scales";

class ChildrenSankeyGraphUp extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [ { type: 'sankey', data: [], links: [], nodeAlign: 'right', lineStyle: { color: 'gradient', curveness: 0.5 } } ],
            tooltip: { show: true }
        };
        this.chart.setOption(this.option);
    }
    update(items){
        const nodes = [{ name: 'Shaper' }];
        const links = [];
        items.forEach(it => {
            const color = colorByRetransmitPct(it.rxmit ?? 0);
            nodes.push({ name: it.name, itemStyle: { color } });
            links.push({ source: 'Shaper', target: it.name, value: it.value });
        });
        this.option.series[0].data = nodes;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

export class ShaperChildrenUp extends BaseDashlet {
    constructor(slot){
        super(slot);
        this.last=null; this._emptyId = this.id + "_empty";
        this._smooth = new Map();
        this._ALPHA = 0.3; this._ALPHA_RX = 0.2; this._DECAY = 0.15; this._LINGER = 3;
    }
    canBeSlowedDown(){ return true; }
    title(){ return "Shaper Children (Upload)"; }
    tooltip(){ return "<h5>Child Throughput</h5><p>Top child nodes by upload throughput."; }
    subscribeTo(){ return ["TreeSummary"]; }
    buildContainer(){ let b=super.buildContainer(); b.appendChild(this.graphDiv()); return b; }
    setup(){ this.graph = new ChildrenSankeyGraphUp(this.graphDivId()); if (this.last) this.graph.update(this.last); }
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
    _smoothItems(items){
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
                if (linger <= 0 || v < 1) this._smooth.delete(name); else this._smooth.set(name, { v, rx: state.rx, linger });
            }
        }
        let out = Array.from(this._smooth.entries()).map(([name,s]) => ({ name, value: s.v, rxmit: s.rx }));
        out.sort((a,b)=> b.value - a.value);
        return out.slice(0,9);
    }
    onMessage(msg){
        if (msg.event !== "TreeSummary") return;
        let rows = (msg.data || []).slice(1).map(r => {
            const m = r[1] || {};
            const name = m.name || String(r[0]);
            const up = Number((m.current_throughput||[0,0])[1]||0);
            const rxmit = (m.current_tcp_packets && m.current_tcp_packets[1] > 0)
                ? ( (m.current_retransmits?.[1]||0) / Math.max(1, m.current_tcp_packets?.[1]||0) ) * 100.0
                : 0;
            return { name, value: up, rxmit };
        });
        rows = this._smoothItems(rows);
        const hasData = rows.some(r => r.value > 0.5);
        this._showEmpty(!hasData);
        if (hasData) {
            this.last = rows;
            this.graph.update(rows);
        }
    }
}
