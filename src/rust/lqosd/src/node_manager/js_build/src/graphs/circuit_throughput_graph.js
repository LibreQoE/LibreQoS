import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

// Helper to format time as HH:MM:SS
function formatTime(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    return d.toLocaleTimeString();
}

const RING_SIZE = 60 * 5; // 5 Minutes

export class CircuitTotalGraph extends DashboardGraph {
    constructor(id, title) {
        super(id);
        this.title = title;
        this.ringbuffer = new RingBuffer(RING_SIZE);

        // Capture references for closure
        const ringbuffer = this.ringbuffer;
        const formatTimeRef = formatTime;
        const scaleNumberRef = scaleNumber;

        this.option = {
            title: {
                text: this.title,
            },
            grid: { left: '20%' },
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "Download",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }, {
                        name: "Upload",
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
                data: [], // will be set in update()
                axisPointer: {
                    type: 'cross'
                }
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => {
                        return scaleNumber(Math.abs(val), 1);
                    },
                }
            },
            series: [
                {
                    name: 'Download',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: window.graphPalette[0]
                    },
                    symbol: 'none',
                },
                {
                    name: 'Upload',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: window.graphPalette[1]
                    },
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
                formatter: function(params) {
                    if (!params || params.length === 0) return '';
                    const idx = params[0].dataIndex;
                    const ts = ringbuffer.getTimestamp(idx);
                    let s = `<div><b>Time:</b> ${formatTimeRef(ts)}</div>`;
                    for (const p of params) {
                        s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${p.color};"></span>${p.seriesName}: <b>${scaleNumber(Math.abs(p.value))}</b></div>`;
                    }
                    return s;
                }
            },
        }
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.series[0].lineStyle.color = window.graphPalette[0];
        this.option.series[1].lineStyle.color = window.graphPalette[1];
        this.chart.setOption(this.option);
    }

    update(download, upload) {
        this.chart.hideLoading();
        this.ringbuffer.push(download, upload, Date.now());

        let data = this.ringbuffer.series();
        let timestamps = this.ringbuffer.getTimestamps();
        // Format xAxis labels as HH:MM:SS
        let xLabels = timestamps.map(ts => formatTime(ts));

        this.option.series[0].data = data[0];
        this.option.series[1].data = data[1];
        this.option.xAxis.data = xLabels;

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(download, upload, timestamp) {
        this.data[this.head][0] = download;
        this.data[this.head][1] = 0.0 - upload;
        this.data[this.head][2] = timestamp || Date.now();
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [
            [], []
        ];
        for (let i=this.head; i<this.size; i++) {
            for (let j=0; j<2; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        for (let i=0; i<this.head; i++) {
            for (let j=0; j<2; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        return result;
    }

    getTimestamps() {
        let result = [];
        for (let i=this.head; i<this.size; i++) {
            result.push(this.data[i][2]);
        }
        for (let i=0; i<this.head; i++) {
            result.push(this.data[i][2]);
        }
        return result;
    }

    getTimestamp(idx) {
        // idx is relative to the current ringbuffer order
        // reconstruct the logical index
        let logicalIdx = (this.head + idx) % this.size;
        return this.data[logicalIdx][2];
    }
}