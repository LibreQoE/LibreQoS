import {isDarkMode} from "../helpers/dark_mode";

export class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        this.chart = echarts.init(this.dom);
        this.chart.showLoading();
        this.option = {};
    }
}