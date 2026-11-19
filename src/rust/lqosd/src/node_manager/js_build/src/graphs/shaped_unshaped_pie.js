import {DashboardGraph} from "./dashboard_graph";
import {t} from "../i18n/index.js";

export class ShapedUnshapedPie extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    type: 'pie',
                    radius: '50%',
                    data: [
                        { name: t("graph.pie.unmapped"), value: 0 },
                        { name: t("graph.pie.mapped"), value: 0 },
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
