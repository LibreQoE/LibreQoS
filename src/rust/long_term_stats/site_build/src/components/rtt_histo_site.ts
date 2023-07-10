import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class RttHistoSite implements Component {
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
    }

    ontick(): void {
    }

    onmessage(event: any): void {
        if (event.msg == "RttChartSite") {
            //console.log(event);
            this.download = [];
            this.x = [];
            for (let i = 0; i < event.RttChartSite.histogram.length; i++) {
                this.download.push(event.RttChartSite.histogram[i]);
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
                            name: 'ms',
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