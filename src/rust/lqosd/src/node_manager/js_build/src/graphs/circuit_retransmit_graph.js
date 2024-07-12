import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class CircuitRetransmitGraph extends DashboardGraph {
    constructor(id, title) {
        super(id);
        this.title = title;
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push(i);
        }

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
                            color: "green"
                        }
                    }, {
                        name: "Upload",
                        icon: 'circle',
                        itemStyle: {
                            color: "orange"
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
                        color: 'green',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Upload',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'green',
                    },
                    symbol: 'none',
                },
            ],
            tooltip: {
                trigger: 'item',
            },
        }
        this.option && this.chart.setOption(this.option);
    }

    update(download, upload) {
        this.chart.hideLoading();
        this.ringbuffer.push(download, upload);

        let data = this.ringbuffer.series();
        this.option.series[0].data = data[0];
        this.option.series[1].data = data[1];

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0, 0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(download, upload) {
        this.data[this.head][0] = download;
        this.data[this.head][1] = 0.0 - upload;
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
}