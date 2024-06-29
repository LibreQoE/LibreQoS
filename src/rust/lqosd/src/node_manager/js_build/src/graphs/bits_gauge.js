import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

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
                            color: 'orange',
                        },
                        length: '80%',
                        icon: 'path://M2.9,0.7L2.9,0.7c1.4,0,2.6,1.2,2.6,2.6v115c0,1.4-1.2,2.6-2.6,2.6l0,0c-1.4,0-2.6-1.2-2.6-2.6V3.3C0.3,1.9,1.4,0.7,2.9,0.7z',
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
                        color: 'orange',
                    },
                    data: [
                        {
                            name: "UP",
                            value: 0,
                            title: { offsetCenter: ['-40%', '75%'] },
                            detail: { offsetCenter: ['-40%', '95%'] },
                        },
                    ]
                },

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
                        icon: 'path://M2.9,0.7L2.9,0.7c1.4,0,2.6,1.2,2.6,2.6v115c0,1.4-1.2,2.6-2.6,2.6l0,0c-1.4,0-2.6-1.2-2.6-2.6V3.3C0.3,1.9,1.4,0.7,2.9,0.7z',
                        itemStyle: {
                            color: 'red'
                        },
                        width: 4,
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
                        color: 'red',
                    },
                    data: [
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
        this.option.series[1].data[0].value = value2;
        this.option.series[0].min = 0;
        this.option.series[0].max = max1 * 1000000; // Convert to bits
        this.option.series[1].min = 0;
        this.option.series[1].max = max2 * 1000000; // Convert to bits
        this.chart.setOption(this.option);
    }
}