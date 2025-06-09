import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";

const RING_SIZE = 60 * 5; // 5 Minutes

function formatTime(ts) {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', { hour12: false });
}

export class BakeryCircuitsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new BakeryRingBuffer(RING_SIZE);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Circuits", 40)
            .build();

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [
                {
                    name: "Active Circuits",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[0]
                    }
                }, {
                    name: "Lazy Circuits",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[2]
                    }
                }
            ],
            textStyle: {
                color: '#aaa'
            },
        };
        this.option.series = [
            {
                name: 'Lazy Circuits',
                data: [],
                type: 'line',
                stack: 'circuits',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[2],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[2]
                },
            },
            {
                name: 'Active Circuits',
                data: [],
                type: 'line',
                stack: 'circuits',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[0],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[0]
                }
            },
        ];

        // Add axisPointer and tooltip with time display
        this.option.tooltip = {
            trigger: 'axis',
            axisPointer: {
                type: 'cross',
                link: [{ xAxisIndex: 'all' }],
                label: {
                    backgroundColor: '#6a7985'
                }
            },
            formatter: (params) => {
                // params is an array of series data at the hovered index
                // Find the timestamp from the ringbuffer
                if (!params || params.length === 0) return '';
                const idx = params[0].dataIndex;
                const ts = this.ringbuffer.getTimestamp(idx);
                let s = `<div><b>Time:</b> ${formatTime(ts)}</div>`;
                for (const p of params) {
                    s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${p.color};"></span>${p.seriesName}: <b>${p.value}</b></div>`;
                }
                return s;
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[2];
        this.option.series[0].lineStyle.color = window.graphPalette[2];
        this.option.series[0].areaStyle.color = window.graphPalette[2];
        this.option.series[1].lineStyle.color = window.graphPalette[0];
        this.option.series[1].areaStyle.color = window.graphPalette[0];

        this.chart.setOption(this.option);
    }

    update(active, lazy) {
        this.chart.hideLoading();
        this.ringbuffer.push(active, lazy, Date.now());

        let data = this.ringbuffer.series();
        this.option.series[0].data = data[0]; // Lazy
        this.option.series[1].data = data[1]; // Active

        this.chart.setOption(this.option);
    }
}

class BakeryRingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0, 0]); // lazy, active, timestamp
        }
        this.head = 0;
        this.data = data;
    }

    push(active, lazy, timestamp) {
        this.data[this.head][0] = lazy;
        this.data[this.head][1] = active;
        this.data[this.head][2] = timestamp || Date.now();
        this.head += 1;
        this.head %= this.size;
    }

    getTimestamp(idx) {
        // idx is the logical index in the chart (0 = oldest)
        // Map to physical index in ring buffer
        let physical = (this.head + idx) % this.size;
        return this.data[physical][2];
    }

    series() {
        let result = [[], []]; // lazy, active
        for (let i=this.head; i<this.size; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        for (let i=0; i<this.head; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        return result;
    }
}