import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';
import { request_rtt_histogram } from "../../wasm/wasm_pipe";

export class RttHisto implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    download: any;
    x: any;
    chartMade: boolean = false;

    constructor() {
        this.div = document.getElementById("rttHisto") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
        request_rtt_histogram(window.graphPeriod);
    }

    ontick(): void {
        request_rtt_histogram(window.graphPeriod);
    }

    onmessage(event: any): void {
        if (event.msg == "RttHistogram") {
            //console.log(event);
            this.download = [];
            this.x = [];
            for (let i = 0; i < event.RttHistogram.histogram.length; i++) {
                this.download.push(event.RttHistogram.histogram[i]);
                this.x.push(i * 10);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "TCP Round-Trip Time Histogram" },
                        xAxis: {
                            type: 'category',
                            data: this.x,                            
                        },
                        yAxis: {
                            type: 'value',
                            name: 'frequency',
                        },
                        series: [
                            {
                                name: "RTT",
                                type: "bar",
                                data: this.download,
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