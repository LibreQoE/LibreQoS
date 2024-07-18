import {isDarkMode} from "../helpers/dark_mode";

export class DashboardGraph {
    constructor(id) {
        let theme = "macarons";
        if (isDarkMode()) theme = "dark";
        this.id = id;
        this.dom = document.getElementById(id);
        this.chart = echarts.init(this.dom, theme);
        this.chart.showLoading();
        this.option = {};
    }
}