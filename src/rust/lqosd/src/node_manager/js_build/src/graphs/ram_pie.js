import {DashboardGraph} from "./dashboard_graph";
import {t} from "../i18n/index.js";

export class RamPie extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    type: 'pie',
                    radius: '50%',
                    data: [
                        { name: t("graph.pie.free"), value: 0 },
                        { name: t("graph.pie.used"), value: 0 }
                    ],
                    label: {
                        color: '#aaa'
                    }
                }
            ],
            tooltip: {
                trigger: 'item',
            },
        }
        this.option && this.chart.setOption(this.option);
    }

    update(free, used) {
        this.chart.hideLoading();
        this.option.series[0].data[0].value = free;
        this.option.series[0].data[1].value = used;
        this.chart.setOption(this.option);
    }
}