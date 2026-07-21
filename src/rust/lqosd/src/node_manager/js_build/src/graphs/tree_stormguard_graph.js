import {DashboardGraph} from "./dashboard_graph";

function line(name, data, xAxisIndex, yAxisIndex, color, extra = {}) {
    return {
        name,
        type: "line",
        data,
        xAxisIndex,
        yAxisIndex,
        showSymbol: false,
        connectNulls: false,
        animation: false,
        lineStyle: {width: 2, color, ...(extra.lineStyle || {})},
        ...extra,
    };
}

function seriesData(points, field) {
    return points.map((point) => [point.timestamp, point[field]]);
}

export class TreeStormguardGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.points = [];
        this.chart.hideLoading();
        this.render([]);
    }

    render(points) {
        this.points = points;
        const markers = points
            .filter((point) => point.marker)
            .map((point) => ({
                value: [point.marker.timestamp, point.marker.targetMbps ?? point.queueMbps],
                name: `${point.marker.action || "action"}: ${point.marker.outcome || "unknown"}`,
                itemStyle: {
                    color: point.marker.outcome === "failed" ? "#dc3545"
                        : point.marker.outcome === "dry_run" ? "#ffc107"
                            : "#0d6efd",
                },
            }));
        const palette = window.graphPalette || ["#0d6efd", "#20c997", "#6c757d", "#fd7e14", "#6f42c1"];
        const commonXAxis = {
            type: "time",
            axisLabel: {hideOverlap: true},
            axisPointer: {show: true},
        };
        const option = {
            animation: false,
            tooltip: {trigger: "axis", axisPointer: {type: "cross", link: [{xAxisIndex: "all"}]}},
            legend: {top: 0, type: "scroll"},
            grid: [
                {left: 62, right: 24, top: 42, height: "25%"},
                {left: 62, right: 24, top: "39%", height: "25%"},
                {left: 62, right: 24, top: "72%", height: "18%"},
            ],
            xAxis: [
                {...commonXAxis, gridIndex: 0, axisLabel: {show: false}},
                {...commonXAxis, gridIndex: 1, axisLabel: {show: false}},
                {...commonXAxis, gridIndex: 2},
            ],
            yAxis: [
                {type: "value", gridIndex: 0, name: "Mbps", min: 0},
                {type: "value", gridIndex: 1, name: "RTT ms", min: 0},
                {type: "value", gridIndex: 2, name: "Decision"},
                {type: "value", gridIndex: 2, name: "Cooldown s", position: "right", min: 0},
            ],
            series: [
                line("Queue limit", seriesData(points, "queueMbps"), 0, 0, palette[0]),
                line("Throughput", seriesData(points, "throughputMbps"), 0, 0, palette[1]),
                line("Minimum", seriesData(points, "minMbps"), 0, 0, palette[2], {lineStyle: {type: "dashed"}}),
                line("Maximum", seriesData(points, "maxMbps"), 0, 0, palette[3], {lineStyle: {type: "dashed"}}),
                {
                    name: "Actions",
                    type: "scatter",
                    data: markers,
                    xAxisIndex: 0,
                    yAxisIndex: 0,
                    symbol: "diamond",
                    symbolSize: 12,
                    tooltip: {formatter: (item) => item.data.name},
                },
                line("Effective RTT", seriesData(points, "effectiveRttMs"), 1, 1, palette[0]),
                line("Passive RTT", seriesData(points, "passiveRttMs"), 1, 1, palette[1]),
                line("Active RTT", seriesData(points, "activeRttMs"), 1, 1, palette[3]),
                line("Baseline", seriesData(points, "baselineRttMs"), 1, 1, palette[2], {lineStyle: {type: "dashed"}}),
                line("Delay", seriesData(points, "delayMs"), 1, 1, palette[4]),
                line("Score", seriesData(points, "decisionScore"), 2, 2, palette[0]),
                line("Cooldown", seriesData(points, "cooldownSeconds"), 2, 3, palette[3]),
            ],
        };
        this.option = option;
        this.chart.setOption(option, true);
    }

    onThemeChange() {
        super.onThemeChange();
        this.render(this.points);
    }
}
