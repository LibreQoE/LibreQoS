import { request_site_stack } from "../../wasm/wasm_pipe";
import { scaleNumber } from "../helpers";
import { Component } from "./component";
import * as echarts from 'echarts';

export class SiteStackChart implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    chartMade: boolean = false;
    siteId: string;
    counter: number = 0;

    constructor(siteId: string) {
        this.siteId = siteId;
        this.div = document.getElementById("siteStack") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
        request_site_stack(window.graphPeriod, this.siteId);
    }

    ontick(): void {
        this.counter++;
        request_site_stack(window.graphPeriod, this.siteId);
    }

    onmessage(event: any): void {
        if (event.msg == "SiteStack") {
            let series: echarts.SeriesOption[] = [];

            // Iterate all provides nodes and create a set of series for each,
            // providing upload and download banding per node.
            let x: any[] = [];
            let first = true;
            let legend: string[] = [];
            for (let i = 0; i < event.SiteStack.nodes.length; i++) {
                let node = event.SiteStack.nodes[i];
                if (node.node_name != "Root") {
                    legend.push(node.node_name);
                    //legend.push(node.node_name + " UL");
                    //console.log(node);

                    let d: number[] = [];
                    let u: number[] = [];
                    let l: number[] = [];
                    for (let j = 0; j < node.down.length; j++) {
                        if (first) x.push(node.down[j].date);
                        d.push(node.down[j].value * 8.0);
                        u.push(node.down[j].u * 8.0);
                        l.push(node.down[j].l * 8.0);
                    }
                    if (first) first = false;

                    let val: echarts.SeriesOption = {
                        name: node.node_name,
                        type: "line",
                        data: d,
                        symbol: 'none',
                        stack: 'download',
                        areaStyle: {},
                    };

                    series.push(val);

                    // Do the same for upload
                    d = [];
                    u = [];
                    l = [];
                    for (let j = 0; j < node.down.length; j++) {
                        d.push(0.0 - (node.up[j].value * 8.0));
                        u.push(0.0 - (node.up[j].u * 8.0));
                        l.push(0.0 - (node.up[j].l * 8.0));
                    }

                    val = {
                        name: node.node_name,
                        type: "line",
                        data: d,
                        symbol: 'none',
                        stack: 'upload',
                        areaStyle: {},
                        label: { show: false }
                    };

                    series.push(val);
                }
            }

            if (!this.chartMade) {
                this.myChart.hideLoading();
                var option: echarts.EChartsOption;
                this.myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "Child Node Throughput (Bits)" },
                        tooltip: { 
                            trigger: "axis",
                            formatter: function (params: any) {
                                console.log(params);
                                let result = "";
                                for (let i = 0; i < params.length; i+=2) {
                                    let siteName = params[i].seriesName;
                                    siteName += " (⬇️" + scaleNumber(params[i].value) + " / ⬆️" + scaleNumber(0.0 - params[i+1].value) + ")";
                                    result += `${siteName}<br />`;
                                }
                                return result;
                                //return `${params.seriesName}<br />
                                //    ${params.name}: ${params.data.value}<br />
                                //    ${params.data.name1}: ${params.data.value1}`;
                            }
                        },
                        legend: {
                            orient: "vertical",
                            right: 0,
                            top: "bottom",
                            data: legend,
                            textStyle: { fontSize: 8 }
                        },
                        xAxis: {
                            type: 'category',
                            data: x,
                            position: 'top',
                        },
                        yAxis: {
                            type: 'value',
                            axisLabel: {
                                formatter: function (val: number) {
                                    return scaleNumber(Math.abs(val));
                                }
                            }
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