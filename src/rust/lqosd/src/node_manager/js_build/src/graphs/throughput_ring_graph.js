import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

function formatTime(ts) {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', { hour12: false });
}

export class ThroughputRingBufferGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Throughput (bps)", 40)
            .build();

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [
                {
                    name: "Mapped Traffic",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[0]
                    }
                }, {
                    name: "Unmapped Traffic",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[1]
                    }
                }
            ],
            textStyle: {
                color: '#aaa'
            },
        };
        this.option.series = [
            {
                name: 'shaped0',
                data: [],
                type: 'line',
                stack: 'shaped',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[0],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[0]
                },
            },
            {
                name: 'Mapped Traffic',
                data: [],
                type: 'line',
                stack: 'shaped',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[0],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[0]
                }

            },
            {
                name: 'unshaped0',
                data: [],
                type: 'line',
                lineStyle: {
                    color: window.graphPalette[1],
                },
                symbol: 'none',
            },
            {
                name: 'Unmapped Traffic',
                data: [],
                type: 'line',
                lineStyle: {
                    color: window.graphPalette[1],
                },
                symbol: 'none',
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
                    s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${p.color};"></span>${p.seriesName}: <b>${scaleNumber(Math.abs(p.value))}</b></div>`;
                }
                return s;
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.series[0].lineStyle.color = window.graphPalette[0];
        this.option.series[0].areaStyle.color = window.graphPalette[0];
        this.option.series[1].lineStyle.color = window.graphPalette[0];
        this.option.series[1].areaStyle.color = window.graphPalette[0];
        this.option.series[2].lineStyle.color = window.graphPalette[1];
        this.option.series[3].lineStyle.color = window.graphPalette[1];

        this.chart.setOption(this.option);
    }

    update(shaped, unshaped) {
        this.chart.hideLoading();
        this.ringbuffer.push(shaped, unshaped, Date.now());

        let data = this.ringbuffer.series();
        this.option.series[0].data = data[0];
        this.option.series[1].data = data[1];
        this.option.series[2].data = data[2];
        this.option.series[3].data = data[3];

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0, 0, 0, 0]); // Add timestamp as 5th element
        }
        this.head = 0;
        this.data = data;
    }

    push(shaped, unshaped, timestamp) {
        this.data[this.head][1] = shaped.down;
        this.data[this.head][0] = 0.0 - shaped.up;
        this.data[this.head][2] = unshaped.down;
        this.data[this.head][3] = 0.0 - unshaped.up;
        this.data[this.head][4] = timestamp || Date.now();
        this.head += 1;
        this.head %= this.size;
    }

    getTimestamp(idx) {
        // idx is the logical index in the chart (0 = oldest)
        // Map to physical index in ring buffer
        let physical = (this.head + idx) % this.size;
        return this.data[physical][4];
    }

    series() {
        let result = [
            [], [], [], []
        ];
        for (let i=this.head; i<this.size; i++) {
            for (let j=0; j<4; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        for (let i=0; i<this.head; i++) {
            for (let j=0; j<4; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        return result;
    }
}
