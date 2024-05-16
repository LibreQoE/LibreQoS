import html from './dashboard.html';
import {Page} from "../../page";
import {requestFlowCount, requestThroughput} from "../../requests";
import {scaleNumber} from "../../scaling";
import * as echarts from 'echarts';

export class DashboardPage extends Page {
    throughputBuffer: ThroughputRingBuffer;
    myChart: echarts.ECharts;
    chartMade: boolean;

    constructor() {
        super();
        this.fillContent(html);
        this.throughputBuffer = new ThroughputRingBuffer();
        this.chartMade = false;
    }

    wireup() {
        requestFlowCount();
        requestThroughput();

        // This is a fake await for after the HTML has loaded
        window.setTimeout(() => {
            let div = document.getElementById("throughputs") as HTMLDivElement;
            this.myChart = echarts.init(div);
            this.myChart.showLoading();
        }, 1);
    }

    onmessage(event: any): void {
        switch (event.type) {
            case "FlowCount": {
                let target = document.getElementById("flowCounter");
                if (target) {
                    target.innerHTML = event.count;
                }
            } break;
            case "Throughput": {
                let target = document.getElementById("pps");
                if (target) {
                    target.innerHTML = scaleNumber(event.pps[0]) + " / " + scaleNumber(event.pps[1])
                }

                target = document.getElementById("bps");
                if (target) {
                    target.innerHTML = scaleNumber(event.bps[0]) + " / " + scaleNumber(event.bps[1])
                }

                this.throughputBuffer.push(event);
                this.doThroughputChart();
            } break;
        }
    }

    ontick(): void {
        requestFlowCount();
        requestThroughput();
    }

    anchor(): string {
        return "dashboard";
    }

    doThroughputChart(): void {
        if (!this.myChart) return;
        if (!this.chartMade) {
            this.chartMade = true;
            this.myChart.hideLoading();
        }

        let rawData = this.throughputBuffer.getSeries();

        let option = {
            xAxis: {
                type: 'category',
                data: rawData[0],
            },
            yAxis: {
                type: 'value'
            },
            series: [
                {
                    data: rawData[1],
                    type: 'line',
                    name: "BPS",
                },
                {
                    data: rawData[2],
                    type: 'line',
                    name: "BPSU",
                },
                {
                    data: rawData[3],
                    type: 'line',
                    name: "Shaped",
                },
                {
                    data: rawData[4],
                    type: 'line',
                    name: "ShapedU",
                }
            ],
            grid: {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0
            }
        };
        option && this.myChart.setOption(option);
    }
}

const MAX_ENTRIES: number = 300;

class ThroughputRingBuffer {
    entries: any[];
    head: number;

    constructor() {
        this.entries = [];
        for (let i=0; i<MAX_ENTRIES; i++) {
            this.entries.push({
                bps: [0, 0],
                shaped: [0, 0],
            })
        }
        this.head = 0;
    }

    push(event: any): void {
        this.entries[this.head].bps[0] = event.bps[0];
        this.entries[this.head].bps[1] = event.bps[1];
        this.entries[this.head].shaped[0] = event.shaped[0];
        this.entries[this.head].shaped[1] = event.shaped[1];
        this.head += 1;
        if (this.head > MAX_ENTRIES) {
            this.head = 0;
        }
    }

    getSeries(): number[][] {
        let result = [];

        let xAxis = [];
        let bpsDown = [];
        let bpsUp = []
        let shapedDown = [];
        let shapedUp = []
        let count = 0;

        for (let i=this.head; i<MAX_ENTRIES; i++) {
            xAxis.push(count);
            count++;
            bpsDown.push(this.entries[i].bps[0]);
            bpsUp.push(this.entries[i].bps[1]);
            shapedDown.push(0 - this.entries[i].shaped[0]);
            shapedUp.push(0 - this.entries[i].shaped[1]);
        }
        for (let i=0; i<this.head; i++) {
            xAxis.push(count);
            count++;
            bpsDown.push(this.entries[i].bps[0]);
            bpsUp.push(this.entries[i].bps[1]);
            shapedDown.push(0 - this.entries[i].shaped[0]);
            shapedUp.push(0 - this.entries[i].shaped[1]);
        }
        result.push(xAxis);
        result.push(bpsDown);
        result.push(bpsUp);
        result.push(shapedDown);
        result.push(shapedUp);

        return result;
    }
}