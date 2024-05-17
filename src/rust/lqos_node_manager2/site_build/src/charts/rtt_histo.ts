import * as echarts from 'echarts';
import {initEchartsWithTheme} from "./echarts_themes";

export class RttHistogram {
    divName: string;
    myChart: echarts.ECharts;
    values: number[];

    constructor(divName: string) {
        this.divName = divName;
        let div = document.getElementById(divName) as HTMLDivElement;
        this.myChart = initEchartsWithTheme(div);
        this.myChart.showLoading();
        this.values = [];
        for (let i=0; i<20; i++) {
            this.values.push(0);
        }
    }

    onMessage(entries: number[]) {
        this.values = entries;
        this.plotGraph();
    }

    plotGraph() {
        this.myChart.hideLoading();
        let option = {
            animation: false, // No animations, please!
            xAxis: {
                type: 'category',
                data: [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200],
            },
            yAxis: {
                type: 'value'
            },
            series: [
                {
                    data: this.values,
                    type: 'bar'
                }
            ],
            grid: {
                left: 50,
                top: 20,
                right: 5,
                bottom: 50,
            }
        };
        option && this.myChart.setOption(option);
    }
}