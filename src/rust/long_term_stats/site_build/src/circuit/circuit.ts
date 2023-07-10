import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { CircuitInfo } from '../components/circuit_info';
import { ThroughputCircuitChart } from '../components/throughput_circuit';
import { RttChartCircuit } from '../components/rtt_circuit';
import { request_ext_device_info, request_ext_snr_graph, request_ext_capacity_graph } from "../../wasm/wasm_pipe";
import * as echarts from 'echarts';
import { scaleNumber } from '../helpers';
import { CircuitBreadcrumbs } from '../components/circuit_breadcrumbs';

export class CircuitPage implements Page {
    menu: MenuPage;
    components: Component[];
    circuitId: string;

    constructor(circuitId: string) {
        this.circuitId = circuitId;
        this.menu = new MenuPage("sitetreeDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new CircuitInfo(this.circuitId),
            new ThroughputCircuitChart(this.circuitId),
            new RttChartCircuit(this.circuitId),
            new CircuitBreadcrumbs(this.circuitId),
        ];
    }

    wireup() {
        this.components.forEach(component => {
            component.wireup();
        });
        request_ext_device_info(this.circuitId);
    }

    ontick(): void {
        this.menu.ontick();
        this.components.forEach(component => {
            component.ontick();
        });
    }

    onmessage(event: any) {
        if (event.msg) {
            this.menu.onmessage(event);

            this.components.forEach(component => {
                component.onmessage(event);
            });

            if (event.msg == "DeviceExt") {
                //console.log(event.DeviceExt.data);
                let div = document.getElementById("ext") as HTMLDivElement;
                let html = "";

                for (let i=0; i<event.DeviceExt.data.length; i++) {
                    let d = event.DeviceExt.data[i];
                    html += "<div class='row'>";

                    html += "<div class='col-4'>";
                    html += "<div class='card'>";
                    html += "<div class='card-body' style='height: 250px'>";
                    html += "<h4>" + d.name + "</h4>";
                    html += "<strong>Status</strong>: " + d.status + "<br>";
                    html += "<strong>Model</strong>: " + d.model + "<br>";
                    html += "<strong>Mode</strong>: " + d.mode + "<br>";
                    html += "<strong>Firmware</strong>: " + d.firmware + "<br>";
                    html += "</div>";
                    html += "</div>";
                    html += "</div>";

                    html += "<div class='col-4'>";
                    html += "<div class='card'>";
                    html += "<div class='card-body' id='extdev_" + d.device_id + "' style='height: 250px'>";
                    html += "<p>Signal/noise graph</p>";
                    html += "</div>";
                    html += "</div>";
                    html += "</div>";
                    request_ext_snr_graph(window.graphPeriod, d.device_id);

                    html += "<div class='col-4'>";
                    html += "<div class='card'>";
                    html += "<div class='card-body' id='extdev_cap_" + d.device_id + "' style='height: 250px'>";
                    html += "<p>Capacity Graph</p>";
                    html += "</div>";
                    html += "</div>";
                    html += "</div>";
                    request_ext_capacity_graph(window.graphPeriod, d.device_id);

                    // End row
                    html += "</div>";
                }

                div.outerHTML = html;
            } else if (event.msg == "DeviceExtSnr") {
                console.log(event);
                let div = document.getElementById("extdev_" + event.DeviceExtSnr.device_id) as HTMLDivElement;

                let sig: number[] = [];
                let n: number[] = [];
                let x: any[] = [];

                for (let i=0; i<event.DeviceExtSnr.data.length; i++) {
                    let d = event.DeviceExtSnr.data[i];
                    sig.push(d.signal);
                    n.push(d.noise);
                    x.push(d.date);
                }

                let series: echarts.SeriesOption[] = [];
                let signal: echarts.SeriesOption = {
                    name: "Signal",
                    type: "line",
                    data: sig,
                    symbol: 'none',
                };
                let noise: echarts.SeriesOption = {
                    name: "Noise",
                    type: "line",
                    data: n,
                    symbol: 'none',
                };
                series.push(signal);
                series.push(noise);

                let myChart: echarts.ECharts = echarts.init(div);
                var option: echarts.EChartsOption;
                myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "Signal/Noise" },
                        legend: {
                            orient: "horizontal",
                            right: 10,
                            top: "bottom",
                        },
                        xAxis: {
                            type: 'category',
                            data: x,
                        },
                        yAxis: {
                            type: 'value',
                            name: 'dB',
                        },
                        series: series
                    })
                );
                option && myChart.setOption(option);
            } else if (event.msg == "DeviceExtCapacity") {
                console.log(event);
                let div = document.getElementById("extdev_cap_" + event.DeviceExtCapacity.device_id) as HTMLDivElement;

                let down: number[] = [];
                let up: number[] = [];
                let x: any[] = [];

                for (let i=0; i<event.DeviceExtCapacity.data.length; i++) {
                    let d = event.DeviceExtCapacity.data[i];
                    down.push(d.dl);
                    up.push(d.ul);
                    x.push(d.date);
                }

                let series: echarts.SeriesOption[] = [];
                let signal: echarts.SeriesOption = {
                    name: "Download",
                    type: "line",
                    data: down,
                    symbol: 'none',
                };
                let noise: echarts.SeriesOption = {
                    name: "Upload",
                    type: "line",
                    data: up,
                    symbol: 'none',
                };
                series.push(signal);
                series.push(noise);

                let myChart: echarts.ECharts = echarts.init(div);
                var option: echarts.EChartsOption;
                myChart.setOption<echarts.EChartsOption>(
                    (option = {
                        title: { text: "Estimated Capacity" },
                        legend: {
                            orient: "horizontal",
                            right: 10,
                            top: "bottom",
                        },
                        xAxis: {
                            type: 'category',
                            data: x,
                        },
                        yAxis: {
                            type: 'value',
                            name: 'Mbps',
                            axisLabel: {
                                formatter: function (val: number) {
                                    return scaleNumber(Math.abs(val), 0);
                                }
                            }
                        },
                        series: series
                    })
                );
                option && myChart.setOption(option);
            }
        }
    }
}
