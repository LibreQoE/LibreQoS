import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class QueueStatsTotalGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push('');
        }

        this.option = {
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "ECN Marks",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }, {
                        name: "Cake Drops",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[1]
                        }
                    }
                ],
                textStyle: {
                    color: '#aaa'
                },
            },
            xAxis: {
                type: 'category',
                data: xaxis,
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => {
                        return scaleNumber(Math.abs(val), 0);
                    },
                }
            },
            series: [
                {
                    name: 'ECN Marks',
                    data: [],
                    type: 'line',
                    lineStyle: { color: window.graphPalette[0] },
                    symbol: 'none',
                },
                {
                    name: 'ECN Marks Up',
                    data: [],
                    type: 'line',
                    lineStyle: { color: window.graphPalette[0] },
                    symbol: 'none',
                },
                {
                    name: 'Cake Drops',
                    data: [],
                    type: 'line',
                    lineStyle: { color: window.graphPalette[1] },
                    symbol: 'none',
                },
                {
                    name: 'Cake Drops Up',
                    data: [],
                    type: 'line',
                    lineStyle: { color: window.graphPalette[1] },
                    symbol: 'none',
                },
            ],
            tooltip: {
                trigger: 'axis',
                axisPointer: {
                    type: 'cross',
                    link: [{ xAxisIndex: 'all' }],
                    label: {
                        backgroundColor: '#6a7985'
                    }
                },
                formatter: (params) => {
                    // params is an array for axis trigger
                    const idx = params[0].dataIndex;
                    const time = this.ringbuffer.getTimestamp(idx);
                    const date = new Date(time);
                    const hhmmss = date.toLocaleTimeString('en-US', { hour12: false });
                    let tooltip = `<b>Time:</b> ${hhmmss}<br/>`;
                    params.forEach((item) => {
                        tooltip += `<span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${item.color};"></span> ${item.seriesName}: <b>${item.value}</b><br/>`;
                    });
                    return tooltip;
                }
            },
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
        this._seriesOnly = { series: this.option.series };
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.series[0].lineStyle.color = window.graphPalette[0];
        this.option.series[1].lineStyle.color = window.graphPalette[0];
        this.option.series[2].lineStyle.color = window.graphPalette[1];
        this.option.series[3].lineStyle.color = window.graphPalette[1];
    }

    update(marks, drops) {
        this.chart.hideLoading();
        this.ringbuffer.push(marks, drops);
    
        const series = this.ringbuffer.series();
        for (let i=0; i<this.option.series.length; i++) {
            this.option.series[i].data = series[i];
        }

        // Tooltip already provides timestamp; keep x-axis labels empty to avoid per-tick allocations.
        this.chart.setOption(this._seriesOnly, false, true);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push({ marks: { down: 0, up: 0 }, drops: { down: 0, up: 0 }, timestamp: Date.now() });
        }
        this.head = 0;
        this.data = data;
        this._seriesCache = [
            new Array(size).fill(0),
            new Array(size).fill(0),
            new Array(size).fill(0),
            new Array(size).fill(0),
        ];
    }

    push(marks, drops) {
        const entry = this.data[this.head];
        entry.marks.down = Number(marks?.down || 0);
        entry.marks.up = Number(marks?.up || 0);
        entry.drops.down = Number(drops?.down || 0);
        entry.drops.up = Number(drops?.up || 0);
        entry.timestamp = Date.now();
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        const out = this._seriesCache;
        let idx = 0;
        for (let i=this.head; i<this.size; i++) {
            const e = this.data[i];
            out[0][idx] = e.marks.down;
            out[1][idx] = 0 - e.marks.up;
            out[2][idx] = e.drops.down;
            out[3][idx] = 0 - e.drops.up;
            idx++;
        }
        for (let i=0; i<this.head; i++) {
            const e = this.data[i];
            out[0][idx] = e.marks.down;
            out[1][idx] = 0 - e.marks.up;
            out[2][idx] = e.drops.down;
            out[3][idx] = 0 - e.drops.up;
            idx++;
        }
        return out;
    }
    
    // Get timestamp for a given index in the logical ring order
    getTimestamp(idx) {
        const realIdx = (this.head + idx) % this.size;
        return this.data[realIdx].timestamp;
    }
}
