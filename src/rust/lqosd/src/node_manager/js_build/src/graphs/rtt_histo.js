import {DashboardGraph} from "./dashboard_graph";

export class RttHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
        for (let i=0; i<20; i++) {
            d.push({
                value: i,
                itemStyle: {color: lerpGreenToRedViaOrange(20-i, 20)},
            });
            axis.push(i.toString());
        }
        this.option = {
            xAxis: {
                type: 'category',
                data: axis,
            },
            yAxis: {
                type: 'value',
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
        for (let i=0; i<20; i++) {
            this.option.series.data[i].value = rtt[i];
        }
        this.chart.setOption(this.option);
    }
}

function lerpGreenToRedViaOrange(value, max) {
    let r = 0;
    let g = 0;
    let b = 0;
    if (value < max / 2) {
        r = 255;
        g = Math.floor(255 * value / (max / 2));
    } else {
        r = Math.floor(255 * (max - value) / (max / 2));
        g = 255;
    }
    return `rgb(${r}, ${g}, ${b})`;
}