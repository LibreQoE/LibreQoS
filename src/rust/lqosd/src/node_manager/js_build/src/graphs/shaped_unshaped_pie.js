import {DashboardGraph} from "./dashboard_graph";

export class ShapedUnshapedPie extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    type: 'pie',
                    radius: '50%',
                    data: [
                        { name: 'Unmapped', value: 0 },
                        { name: 'Mapped', value: 0 },
                    ],
                }
            ],
            tooltip: {
                trigger: 'item',
            },
        }
        this.option && this.chart.setOption(this.option);
    }

    update(shaped, unshaped) {
        this.chart.hideLoading();
        this.option.series[0].data[1].value = shaped;
        this.option.series[0].data[0].value = unshaped - shaped;
        this.chart.setOption(this.option);
    }
}
