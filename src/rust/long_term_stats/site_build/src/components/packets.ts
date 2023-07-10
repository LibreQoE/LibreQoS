import { request_packet_chart } from "../../wasm/wasm_pipe";
import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class PacketsChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    chartMade: boolean = false;

    constructor() {
        this.div = document.getElementById("packetsChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
        request_packet_chart(window.graphPeriod);
    }

    ontick(): void {
        request_packet_chart(window.graphPeriod);
    }

    onmessage(event: any): void {
        if (event.msg == "PacketChart") {
            let series: echarts.SeriesOption[] = [];

            // Iterate all provides nodes and create a set of series for each,
            // providing upload and download banding per node.
            let x: any[] = [];
            let first = true;
            let legend: string[] = [];
            for (let i=0; i<event.PacketChart.nodes.length; i++) {
                let node = event.PacketChart.nodes[i];
                legend.push(node.node_name);
                //legend.push(node.node_name + " UL");
                //console.log(node);

                let d: number[] = [];
                let u: number[] = [];
                let l: number[] = [];
                for (let j=0; j<node.down.length; j++) {
                    if (first) x.push(node.down[j].date);                 
                    d.push(node.down[j].value);
                    u.push(node.down[j].u);
                    l.push(node.down[j].l);
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

                // Do the same for upload
                d = [];
                u = [];
                l = [];
                for (let j=0; j<node.down.length; j++) {
                    d.push(0.0 - node.up[j].value);
                    u.push(0.0 - node.up[j].u);
                    l.push(0.0 - node.up[j].l);
                }

                min = {
                    name: "L",
                    type: "line",
                    data: l,
                    symbol: 'none',
                    stack: 'confidence-band-' + node.node_id,
                    lineStyle: {
                        opacity: 0
                    },
                };
                max = {
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
                val = {
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
                        title: { text: "Packets" },
                        tooltip: { 
                            trigger: "axis",
                            formatter: function (params: any) {
                                let ret = "";
                                for (let i=0; i<params.length; i+=3) {
                                    if (params[i+2].value > 0) {
                                        ret += params[i+2].seriesName + ": " + scaleNumber(Math.abs(params[i+2].value)) + " ⬇️<br/>";
                                    } else {
                                        ret += params[i+2].seriesName + ": " + scaleNumber(Math.abs(params[i+2].value)) + " ⬆️<br/>";
                                    }
                                }
                                return ret;
                            }
                        },
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
                            axisLabel: {
                                formatter: function (val: number) {
                                    return scaleNumber(Math.abs(val));
                                }
                            }
                        },
                        series: series,
                    })
                );
                option && this.myChart.setOption(option);
                // this.chartMade = true;
            }
        }
    }
}