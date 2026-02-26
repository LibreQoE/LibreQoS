import {isDarkMode} from "../helpers/dark_mode";

export class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        if (!this.dom) {
            throw new Error(`DashboardGraph: missing DOM element '${id}'`);
        }
        this.dom.classList.add("muted");

        // If a chart already exists for this DOM (e.g. time period change, zoom open/close),
        // dispose it before re-initializing to prevent memory growth.
        if (typeof echarts !== "undefined" && echarts.getInstanceByDom) {
            const existing = echarts.getInstanceByDom(this.dom);
            if (existing) {
                existing.dispose();
            }
        }

        if (isDarkMode()) {
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
            this.chart = echarts.init(this.dom, 'dark');
        } else {
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
            this.chart = echarts.init(this.dom, 'vintage');
        }
        this.chart.showLoading();
        this.option = {};

        // Apply to the global list of graphs
        if (window.graphList === undefined) {
            window.graphList = [];
        }
        const domId = this.dom.id;
        // De-dupe by DOM id and remove graphs whose DOM has been detached (e.g. closed zoom).
        window.graphList = window.graphList.filter((g) => {
            if (!g || !g.dom || !g.dom.id) return false;
            if (g.dom.id === domId) return false;
            if (typeof g.dom.isConnected === "boolean") {
                return g.dom.isConnected;
            }
            return document.body && document.body.contains ? document.body.contains(g.dom) : true;
        });
        window.graphList.push(this);
    }

    onThemeChange() {
        // Override this if you have to
    }
}
