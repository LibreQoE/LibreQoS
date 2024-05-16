import {GenericRingBuffer} from "./generic_ringbuffer";
import * as echarts from 'echarts';
import {scaleNumber} from "../scaling";
import {initEchartsWithTheme} from "./echarts_themes";
import {currentThemeForChart} from "../darkmode";

export class ThroughputEntry {
    bps: number[];
    shaped: number[];

    default(): ThroughputEntry {
        this.bps = [0,0];
        this.shaped = [0,0];
        return this;
    }
}

export class ThroughputGraph {
    divName: string;
    myChart: echarts.ECharts;
    ringBuffer: GenericRingBuffer<ThroughputEntry>;

    constructor(divName: string) {
        this.divName = divName;
        let div = document.getElementById(divName) as HTMLDivElement;
        this.myChart = initEchartsWithTheme(div);
        this.myChart.showLoading();
        let te = new ThroughputEntry().default();
        this.ringBuffer = new GenericRingBuffer<ThroughputEntry>(300, te);
    }

    onMessage(event: ThroughputEntry) {
        this.ringBuffer.push(event);
        this.plotGraph();
    }

    startingBuffer(events: ThroughputEntry[]) {
        this.ringBuffer.clear();
        for (let i=0; i<events.length; i++) {
            this.ringBuffer.push(events[i]);
        }
    }

    getSeries(): number[][] {
        let result = [];
        let xAxis: number[] = [];
        let bpsUp: number[] = [];
        let bpsDown: number[] = [];
        let shapedUp: number[] = [];
        let shapedDown: number[] = [];

        let counter = 0;
        this.ringBuffer.for_each((e) => {
            xAxis.push(counter);
            bpsUp.push(0 - e.bps[0]);
            bpsDown.push(e.bps[1]);
            shapedUp.push(0 - e.shaped[0]);
            shapedDown.push(e.shaped[1]);
            counter++;
        });

        result.push(xAxis);
        result.push(bpsUp);
        result.push(bpsDown);
        result.push(shapedUp);
        result.push(shapedDown);

        return result;
    }

    plotGraph() {
        this.myChart.hideLoading();
        let rawData = this.getSeries();
        let option = {
            theme: currentThemeForChart(),
            animationDuration: 300,
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                data: [
                    "BPS", "Shaped"
                ]
            },
            xAxis: {
                type: 'category',
                data: rawData[0],
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: function (val: number) {
                        return scaleNumber(Math.abs(val), 1);
                    }
                }
            },
            color: ['rgb(255,160,122)', 'rgb(124,252,0)'],
            series: [
                {
                    data: rawData[3],
                    type: 'line',
                    name: "Shaped",
                    color: 'rgb(124,252,0)',
                    symbol: 'none',
                    stack: "Shaped",
                    areaStyle: {
                        color: 'rgb(124,252,0)',
                        opacity: 0.6,
                    }
                },
                {
                    data: rawData[4],
                    type: 'line',
                    name: "ShapedU",
                    color: 'rgb(124,252,0)',
                    symbol: 'none',
                    stack: "Shaped",
                    areaStyle: {
                        color: 'rgb(124,252,0)',
                        opacity: 0.6,
                    }
                },
                {
                    data: rawData[1],
                    type: 'line',
                    name: "BPS",
                    symbol: 'none',
                    lineStyle: {
                        color: 'rgb(255,160,122)'
                    },
                },
                {
                    data: rawData[2],
                    type: 'line',
                    name: "BPSU",
                    symbol: 'none',
                    lineStyle: {
                        color: 'rgb(255,160,122)'
                    },
                },
            ],
            grid: {
                left: 50,
                top: 0,
                right: 0,
                bottom: 0,
            }
        };
        option && this.myChart.setOption(option);
    }
}