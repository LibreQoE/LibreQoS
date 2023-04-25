import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class PacketsChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    download: any;
    downloadMin: any;
    downloadMax: any;
    upload: any;
    uploadMin: any;
    uploadMax: any;
    x: any;
    chartMade: boolean = false;

    constructor() {
        this.div = document.getElementById("packetsChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
    }

    ontick(): void {
        window.bus.requestPacketChart();
    }

    onmessage(event: any): void {
        if (event.msg == "packetChart") {
            //console.log(event);
            this.download = [];
            this.downloadMin = [];
            this.downloadMax = [];
            this.upload = [];
            this.uploadMin = [];
            this.uploadMax = [];
            this.x = [];
            for (let i = 0; i < event.down.length; i++) {
                this.download.push(event.down[i].value);
                this.downloadMin.push(event.down[i].l);
                this.downloadMax.push(event.down[i].u);
                this.upload.push(0.0 - event.up[i].value);
                this.uploadMin.push(0.0 - event.up[i].l);
                this.uploadMax.push(0.0 - event.up[i].u);
                this.x.push(event.down[i].date);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "Packets" },
                        xAxis: {
                            type: 'category',
                            data: this.x,
                        },
                        yAxis: {
                            type: 'value',
                            axisLabel: {
                                formatter: function (val: number) {
                                    return scaleNumber(Math.abs(val));
                                }
                            }
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
                            // Upload
                            {
                                name: "LU",
                                type: "line",
                                data: this.uploadMin,
                                symbol: 'none',
                                stack: 'confidence-band',
                                lineStyle: {
                                    opacity: 0
                                },
                            },
                            {
                                name: "UU",
                                type: "line",
                                data: this.uploadMax,
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
                                name: "Upload",
                                type: "line",
                                data: this.upload,
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