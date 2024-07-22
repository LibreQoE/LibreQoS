import {isDarkMode} from "../helpers/dark_mode";

export class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        this.dom.classList.add("muted");
        if (isDarkMode()) {
            this.chart = echarts.init(this.dom, 'dark');
            window.graphPalette = [
                '#4992ff',
                '#7cffb2',
                '#fddd60',
                '#ff6e76',
                '#58d9f9',
                '#05c091',
                '#ff8a45',
                '#8d48e3',
                '#dd79ff'
            ];
        } else {
            this.chart = echarts.init(this.dom, 'vintage');
            window.graphPalette = [
                '#d87c7c',
                '#919e8b',
                '#d7ab82',
                '#6e7074',
                '#61a0a8',
                '#efa18d',
                '#787464',
                '#cc7e63',
                '#724e58',
                '#4b565b'
            ];
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