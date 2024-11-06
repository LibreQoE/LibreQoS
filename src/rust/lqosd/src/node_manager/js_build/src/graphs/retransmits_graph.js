import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class RetransmitsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push(i);
        }

        this.option = {
            grid: {
                x: '25%',
            },
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "Download",
                        icon: 'circle',
                    },
                    {
                        name: "Upload",
                        icon: 'circle',
                    },
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
                        return Math.abs(val) + "%";
                    },
                }
            },
            series: [
                {
                    name: 'Download',
                    data: [],
                    type: 'line',
                    symbol: 'none',
                },
                {
                    name: 'Upload',
                    data: [],
                    type: 'line',
                    symbol: 'none',
                },
            ],
            tooltip: {
                trigger: 'item',
            },
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
    }

    update(down, up, tcp_down, tcp_up) {
        if (tcp_down === 0) {
            tcp_down = 1;
        }
        if (tcp_up === 0) {
            tcp_up = 1;
        }
        up = (up / tcp_up) * 100.0; // Percentage
        down = (down / tcp_down) * 100.0; // Percentage
        this.chart.hideLoading();
        this.ringbuffer.push(down, 0 - up);

        let series = this.ringbuffer.series();
        this.option.series[0].data = series[0];
        this.option.series[1].data = series[1];

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(recent, completed) {
        this.data[this.head] = [recent, completed];
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [[], []];
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