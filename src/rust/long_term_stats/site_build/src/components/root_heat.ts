import { Component } from "./component";
import * as echarts from 'echarts';

export class RootHeat implements Component {
    div: HTMLElement;
    myChart: echarts.ECharts;
    counter: number = 0;

    constructor() {
        this.div = document.getElementById("rootHeat") as HTMLElement;
        this.myChart = echarts.init(this.div);
        this.myChart.showLoading();
    }

    wireup(): void {
        window.bus.requestSiteRootHeat();        
    }

    ontick(): void {
        this.counter++;
        if (this.counter % 10 == 0)
            window.bus.requestSiteRootHeat();        
    }

    onmessage(event: any): void {
        if (event.msg == "rootHeat") {
            this.myChart.hideLoading();

            let categories: string[] = [];
            let x: string[] = [];
            let first: boolean = true;
            let count = 0;
            let data: any[] = [];
            let keys: string[] = [];
            for (const key in event.data) {
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
                    for (let i=0; i<event.data[key].length; i++) {
                        x.push(event.data[key][i][0]);
                    }
                }

                // Create all the series entries for this category
                for (let i=0; i<event.data[key].length; i++) {
                    data.push([i, count, event.data[key][i][1].toFixed(1)]);
                }

                count++;
            }

            let series: any[] = [];
            let i = 0;
            series.push({
                name: categories[i],
                type: 'heatmap',
                data: data,
                label: { show: true },
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
            this.myChart.setOption(option);
        }
    }
}