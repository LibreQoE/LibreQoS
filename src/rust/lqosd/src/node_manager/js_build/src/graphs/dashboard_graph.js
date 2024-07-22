import {isDarkMode} from "../helpers/dark_mode";

export class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        if (isDarkMode()) {
            this.chart = echarts.init(this.dom, 'dark');
        } else {
            this.chart = echarts.init(this.dom, 'vintage');
        }
        this.chart.showLoading();
        this.option = {};

        // Apply to the global list of graphs
        if (window.graphList === undefined) {
            window.graphList = [ this ];
        } else {
            window.graphList.push(this);
        }
    }
}