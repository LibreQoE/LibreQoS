import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export const N_ITEMS = 50;

export class RttHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
        for (let i=0; i<N_ITEMS; i++) {
            d.push({
                value: i,
                itemStyle: {color: lerpGreenToRedViaOrange(N_ITEMS-i, N_ITEMS)},
            });
            axis.push((i*10).toString());
        }
        this.max = 1;
        this.rawCounts = []; // Store raw sample counts
        this.option = {
            tooltip: {
                trigger: 'axis',
                axisPointer: {
                    type: 'cross',
                    label: {
                        backgroundColor: '#6a7985'
                    }
                },
                formatter: (params) => {
                    if (params && params[0]) {
                        let index = params[0].dataIndex;
                        let rttMs = index * 10;
                        let rttRangeEnd = rttMs + 10;
                        let samples = this.rawCounts[index] || 0;
                        let percentage = params[0].value.toFixed(2);
                        return `RTT: ${rttMs}-${rttRangeEnd} ms<br/>Samples: ${samples}<br/>Percentage: ${percentage}%`;
                    }
                    return '';
                }
            },
            xAxis: {
                type: 'category',
                data: axis,
                name: "RTT",
                nameLocation: 'middle',
                nameGap: 40,
            },
            yAxis: {
                type: 'value',
                name: "% of Samples",
                nameLocation: 'middle',
                nameGap: 40,
                min: () => 0,
                max: () => this.max,
            },
            series: {
                data: d,
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(rtt) {
        this.chart.hideLoading();
        let sum = 0;
        this.rawCounts = [...rtt]; // Store raw counts
        rtt.forEach((v) => {
            if (v > this.max) {
                this.max = v;
            }
            sum += v;
        })
        for (let i=0; i<N_ITEMS; i++) {
            this.option.series.data[i].value = (rtt[i] / sum) * 100;
        }
        this.chart.setOption(this.option);
    }
}

