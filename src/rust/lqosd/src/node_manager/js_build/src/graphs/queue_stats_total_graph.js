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
    
        let { series, times } = this.ringbuffer.seriesWithTimestamps();
        for (let i=0; i<this.option.series.length; i++) {
            this.option.series[i].data = series[i];
        }
        // Update xAxis with formatted times
        this.option.xAxis.data = times.map(ts => {
            const d = new Date(ts);
            return d.toLocaleTimeString('en-US', { hour12: false });
        });
    
        this.chart.setOption(this.option);
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
    }

    push(marks, drops) {
        this.data[this.head] = {
            marks: marks,
            drops: drops,
            timestamp: Date.now(),
        };
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [[], [], [], []];
        for (let i=this.head; i<this.size; i++) {
            result[0].push(this.data[i].marks.down);
            result[1].push(0 - this.data[i].marks.up);
            result[2].push(this.data[i].drops.down);
            result[3].push(0 - this.data[i].drops.up);
        }
        for (let i=0; i<this.head; i++) {
            result[0].push(this.data[i].marks.down);
            result[1].push(0 - this.data[i].marks.up);
            result[2].push(this.data[i].drops.down);
            result[3].push(0 - this.data[i].drops.up);
        }
        return result;
    }
    
    // Returns both series and timestamps for xAxis
    seriesWithTimestamps() {
        let series = [[], [], [], []];
        let times = [];
        for (let i=this.head; i<this.size; i++) {
            series[0].push(this.data[i].marks.down);
            series[1].push(0 - this.data[i].marks.up);
            series[2].push(this.data[i].drops.down);
            series[3].push(0 - this.data[i].drops.up);
            times.push(this.data[i].timestamp);
        }
        for (let i=0; i<this.head; i++) {
            series[0].push(this.data[i].marks.down);
            series[1].push(0 - this.data[i].marks.up);
            series[2].push(this.data[i].drops.down);
            series[3].push(0 - this.data[i].drops.up);
            times.push(this.data[i].timestamp);
        }
        return { series, times };
    }
    
    // Get timestamp for a given index in the logical ring order
    getTimestamp(idx) {
        const realIdx = (this.head + idx) % this.size;
        return this.data[realIdx].timestamp;
    }
}