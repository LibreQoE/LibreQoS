import { request_node_perf_chart } from "../../wasm/wasm_pipe";
import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class NodeRamChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    chartMade: boolean = false;
    node_id: string;
    node_name: string;

    constructor(node_id: string, node_name: string) {
        this.node_id = node_id;
        this.node_name = node_name;
        this.div = document.getElementById("ramChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
    }

    ontick(): void {
        request_node_perf_chart(window.graphPeriod, this.node_id, this.node_name);
    }

    onmessage(event: any): void {
        if (event.msg == "NodePerfChart") {
            let series: echarts.SeriesOption[] = [];

            // Iterate all provides nodes and create a set of series for each,
            // providing upload and download banding per node.
            let x: any[] = [];
            let first = true;
            let legend: string[] = [];
            for (let i=0; i<event.NodePerfChart.nodes.length; i++) {
                let node = event.NodePerfChart.nodes[i];
                legend.push(node.node_name);
                //console.log(node);

                let ram: number[] = [];
                for (let j=0; j<node.stats.length; j++) {
                    if (first) x.push(node.stats[j].date);                 
                    ram.push(node.stats[j].ram);
                }
                if (first) first = false;

                let val: echarts.SeriesOption = {
                    name: node.node_name,
                    type: "line",
                    data: ram,
                    symbol: 'none',
                };

                series.push(val);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "RAM Usage" },
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
                            name: '%',
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