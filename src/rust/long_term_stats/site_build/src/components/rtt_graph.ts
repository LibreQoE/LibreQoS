import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class RttChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    download: any;
    downloadMin: any;
    downloadMax: any;
    x: any;
    chartMade: boolean = false;

    constructor() {
        this.div = document.getElementById("rttChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
    }

    ontick(): void {
        window.bus.requestRttChart();
    }

    onmessage(event: any): void {
        if (event.msg == "rttChart") {
            //console.log(event);
            this.download = [];
            this.downloadMin = [];
            this.downloadMax = [];
            this.x = [];
            for (let i = 0; i < event.data.length; i++) {
                this.download.push(event.data[i].value);
                this.downloadMin.push(event.data[i].l);
                this.downloadMax.push(event.data[i].u);
                this.x.push(event.data[i].date);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "TCP Round-Trip Time" },
                        xAxis: {
                            type: 'category',
                            data: this.x,
                        },
                        yAxis: {
                            type: 'value',
                            name: 'ms',
                        },
                        series: [
                            {
                                name: "L",
                                type: "line",
                                data: this.downloadMin,
                                symbol: 'none',
                                stack: 'confidence-band',
                                lineStyle: {
                                    opacity: 0
                                },
                            },
                            {
                                name: "U",
                                type: "line",
                                data: this.downloadMax,
                                symbol: 'none',
                                stack: 'confidence-band',
                                lineStyle: {
                                    opacity: 0
                                },
                                areaStyle: {
                                    color: '#ccc'
                                },
                            },
                            {
                                name: "Download",
                                type: "line",
                                data: this.download,
                                symbol: 'none',
                                itemStyle: {
                                    color: '#333'
                                },
                            },
                        ]
                    })
                );
                option && this.myChart.setOption(option);
                // this.chartMade = true;
            }
        }
    }
}