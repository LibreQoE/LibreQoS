import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';
import { request_rtt_chart } from "../../wasm/wasm_pipe";

export class RttChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    chartMade: boolean = false;

    constructor() {
        this.div = document.getElementById("rttChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
        request_rtt_chart(window.graphPeriod);
    }

    ontick(): void {
        request_rtt_chart(window.graphPeriod);
    }

    onmessage(event: any): void {
        if (event.msg == "RttChart") {
            let series: echarts.SeriesOption[] = [];

            // Iterate all provides nodes and create a set of series for each,
            // providing upload and download banding per node.
            let x: any[] = [];
            let first = true;
            let legend: string[] = [];
            for (let i=0; i<event.RttChart.nodes.length; i++) {
                let node = event.RttChart.nodes[i];
                legend.push(node.node_name);
                //console.log(node);

                let d: number[] = [];
                let u: number[] = [];
                let l: number[] = [];
                for (let j=0; j<node.rtt.length; j++) {                    
                    if (first) x.push(node.rtt[j].date);     
                    d.push(node.rtt[j].value);
                    u.push(node.rtt[j].u);
                    l.push(node.rtt[j].l);
                }
                if (first) first = false;

                let min: echarts.SeriesOption = {
                    name: "L",
                    type: "line",
                    data: l,
                    symbol: 'none',
                    stack: 'confidence-band-' + node.node_id,
                    lineStyle: {
                        opacity: 0
                    },
                };
                let max: echarts.SeriesOption = {
                    name: "U",
                    type: "line",
                    data: u,
                    symbol: 'none',
                    stack: 'confidence-band-' + node.node_id,
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: '#ccc'
                    },
                };
                let val: echarts.SeriesOption = {
                    name: node.node_name,
                    type: "line",
                    data: d,
                    symbol: 'none',
                };

                series.push(min);
                series.push(max);
                series.push(val);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "TCP Round-Trip Time" },
                        tooltip: { trigger: "axis" },
                        legend: {
                            orient: "horizontal",
                            right: 10,
                            top: "bottom",
                            data: legend,
                        },
                        xAxis: {
                            type: 'category',
                            data: x,
                        },
                        yAxis: {
                            type: 'value',
                            name: 'ms',
                        },
                        series: series
                    })
                );
                option && this.myChart.setOption(option);
                // this.chartMade = true;
            }
        }
    }
}