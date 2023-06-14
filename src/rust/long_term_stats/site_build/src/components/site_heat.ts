import { Component } from "./component";
import * as echarts from 'echarts';
import { request_site_heat } from "../../wasm/wasm_pipe";

export class SiteHeat implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts | null = null;
    counter: number = 0;
    siteId: string;

    constructor(siteId: string) {
        this.siteId = decodeURI(siteId);
        this.div = document.getElementById("rootHeat") as HTMLElement;
    }

    wireup(): void {
        console.log("SiteHeat wireup");
        request_site_heat(window.graphPeriod, this.siteId);
    }

    ontick(): void {
        console.log("SiteHeat ontick");
        this.counter++;
        if (this.counter % 10 == 0)
            request_site_heat(window.graphPeriod, this.siteId);
    }

    onmessage(event: any): void {
        if (event.msg == "SiteHeat") {

            let categories: string[] = [];
            let x: string[] = [];
            let first: boolean = true;
            let count = 0;
            let data: any[] = [];
            let keys: string[] = [];
            for (const key in event.SiteHeat.data) {
                keys.push(key);
            }
            keys = keys.sort().reverse();
            //console.log(keys);

            for (let j=0; j<keys.length; j++) {
                let key = keys[j];
                categories.push(key);

                // Push the X axis values
                if (first) {
                    first = false;
                    for (let i=0; i<event.SiteHeat.data[key].length; i++) {
                        x.push(event.SiteHeat.data[key][i][0]);
                    }
                }

                // Create all the series entries for this category
                for (let i=0; i<event.SiteHeat.data[key].length; i++) {
                    data.push([i, count, event.SiteHeat.data[key][i][1].toFixed(1)]);
                }

                count++;
            }

            let series: any[] = [];
            let i = 0;
            series.push({
                name: categories[i],
                type: 'heatmap',
                data: data,
                label: { show: true, textStyle: { fontSize: 6 } },
                emphasis: {
                    itemStyle: {
                        shadowBlur: 10,
                        shadowColor: 'rgba(0, 0, 0, 0.5)'
                    }
                    }
            })
            //console.log(series);

            let option = {
                title: { text: "TCP Round-Trip Time by Site" },
                tooltip: {
                    show: false,
                  },
                grid: { height: '50%', top: '10%' },
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
                    inRange : {   
                        color: ['#009000', 'yellow', '#DD2000' ] //From smaller to bigger value ->
                    }
                  },
            };

            if (this.myChart == null) {
                let elements = categories.length;
                let height = (elements * 20) + 250;
                this.div.style.height = height + "px";
                this.myChart = echarts.init(this.div);
            }

            this.myChart.setOption(option);
        }
    }
}