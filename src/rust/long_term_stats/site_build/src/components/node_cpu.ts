import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class NodeCpuChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    chartMade: boolean = false;
    node_id: string;
    node_name: string;

    constructor(node_id: string, node_name: string) {
        this.node_id = node_id;
        this.node_name = node_name;
        this.div = document.getElementById("cpuChart") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
    }

    ontick(): void {
        // Requested by the RAM chart
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
                legend.push(node.node_name + " CPU %");
                legend.push(node.node_name + " Single Core Peak");
                //console.log(node);

                let cpu: number[] = [];
                let cpu_max: number[] = [];
                for (let j=0; j<node.stats.length; j++) {
                    if (first) x.push(node.stats[j].date);                 
                    cpu.push(node.stats[j].cpu);
                    cpu_max.push(node.stats[j].cpu_max);
                }
                if (first) first = false;

                let val: echarts.SeriesOption = {
                    name: node.node_name + " CPU %",
                    type: "line",
                    data: cpu,
                    symbol: 'none',
                };
                let val2: echarts.SeriesOption = {
                    name: node.node_name + " Single Core Peak",
                    type: "line",
                    data: cpu_max,
                    symbol: 'none',
                };

                series.push(val);
                series.push(val2);
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "CPU Usage" },
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