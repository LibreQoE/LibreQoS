import { request_root_heat } from "../../wasm/wasm_pipe";
import { Component } from "./component";
import * as echarts from 'echarts';

export class RootHeat implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts | null = null;
    counter: number = 0;

    constructor() {
        this.div = document.getElementById("rootHeat") as HTMLElement;
    }

    wireup(): void {
        request_root_heat(window.graphPeriod);
    }

    ontick(): void {
        this.counter++;
        if (this.counter % 10 == 0)
            request_root_heat(window.graphPeriod);
    }

    onmessage(event: any): void {
        if (event.msg == "RootHeat") {
            let categories: string[] = [];
            let x: string[] = [];
            let first: boolean = true;
            let count = 0;
            let data: any[] = [];
            let keys: string[] = [];
            for (const key in event.RootHeat.data) {
                keys.push(key);
            }
            keys = keys.sort().reverse();
            //console.log(keys);

            for (let j = 0; j < keys.length; j++) {
                let key = keys[j];
                categories.push(key);

                // Push the X axis values
                if (first) {
                    first = false;
                    for (let i = 0; i < event.RootHeat.data[key].length; i++) {
                        x.push(event.RootHeat.data[key][i][0]);
                    }
                }

                // Create all the series entries for this category
                for (let i = 0; i < event.RootHeat.data[key].length; i++) {
                    data.push([i, count, event.RootHeat.data[key][i][1].toFixed(1)]);
                }

                count++;
            }

            let series: any[] = [];
            let i = 0;
            console.log(categories);
            series.push({
                name: categories[i],
                type: 'heatmap',
                data: data,
                label: { show: true, textStyle: { fontSize: 6 } },
                emphasis: {
                    itemStyle: {
                        shadowBlur: 10,
                        shadowColor: 'rgba(0, 0, 0, 0.5)',                        
                    }
                }
            })
            //console.log(series);

            let option = {
                title: { text: "TCP Round-Trip Time by Site" },
                tooltip: {
                    show: false,
                },
                grid: { height: '95%', top: '10%' },
                xAxis: { type: 'category', data: x, splitArea: { show: true } },
                yAxis: { type: 'category', data: categories, splitArea: { show: true } },
                series: series,
                visualMap: {
                    min: 0,
                    max: 200,
                    calculable: true,
                    type: 'continuous',
                    orient: 'horizontal',
                    left: 'center',
                    top: '2%',
                    inRange: {
                        color: ['#009000', 'yellow', '#DD2000'] //From smaller to bigger value ->
                    }
                },
            };

            if (this.myChart == null) {
                let elements = categories.length;
                let height = (elements * 20) + 50;
                this.div.style.height = height + "px";
                this.myChart = echarts.init(this.div);
            }

            this.myChart.setOption(option);
        }
    }
}