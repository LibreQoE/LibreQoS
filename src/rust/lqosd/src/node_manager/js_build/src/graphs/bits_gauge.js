import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../scaling";

export class BitsPerSecondGauge extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    type: 'gauge',
                    axisLine: {
                        lineStyle: {
                            width: 10,
                            color: [
                                [0.5, 'green'],
                                [0.8, 'orange'],
                                [1, '#fd666d']
                            ]
                        }
                    },
                    pointer: {
                        itemStyle: {
                            color: 'auto'
                        }
                    },
                    axisTick: {
                        distance: -10,
                        length: 8,
                        lineStyle: {
                            color: '#fff',
                            width: 2
                        }
                    },
                    splitLine: {
                        distance: -15,
                        length: 15,
                        lineStyle: {
                            color: '#999',
                            width: 4
                        }
                    },
                    axisLabel: {
                        color: 'inherit',
                        distance: 16,
                        fontSize: 10,
                        formatter: (value) => { return scaleNumber(value, 1); }
                    },
                    detail: {
                        valueAnimation: true,
                        formatter: (value) => { return scaleNumber(value); },
                        color: 'inherit',
                        fontSize: 12,
                    },
                    title: {
                        fontSize: 14,
                        color: 'inherit',
                    },
                    data: [
                        {
                            name: "UP",
                            value: 0,
                            title: { offsetCenter: ['-40%', '75%'] },
                            detail: { offsetCenter: ['-40%', '95%'] },
                        },
                        {
                            name: "DOWN",
                            value: 0,
                            title: { offsetCenter: ['40%', '75%'] },
                            detail: { offsetCenter: ['40%', '95%'] },
                        }
                    ]
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
    }

    update(value1, value2, max1, max2) {
        this.chart.hideLoading();
        this.option.series[0].data[0].value = value1;
        this.option.series[0].data[1].value = value2;
        this.option.series[0].min = 0;
        this.option.series[0].max = Math.max(max1, max2) * 1000000; // Convert to bits
        this.chart.setOption(this.option);
    }
}