import {DashboardGraph} from "./dashboard_graph";
import {t} from "../i18n/index.js";

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
                {type: "value", gridIndex: 1, name: t("graph.stormguard.axis_rtt_ms"), min: 0},
                {type: "value", gridIndex: 2, name: t("graph.stormguard.axis_decision")},
                {type: "value", gridIndex: 2, name: t("graph.stormguard.axis_cooldown_s"), position: "right", min: 0},
            ],
            series: [
                line(t("graph.stormguard.queue_limit"), seriesData(points, "queueMbps"), 0, 0, palette[0]),
                line(t("graph.stormguard.throughput"), seriesData(points, "throughputMbps"), 0, 0, palette[1]),
                line(t("graph.stormguard.minimum"), seriesData(points, "minMbps"), 0, 0, palette[2], {lineStyle: {type: "dashed"}}),
                line(t("graph.stormguard.maximum"), seriesData(points, "maxMbps"), 0, 0, palette[3], {lineStyle: {type: "dashed"}}),
                {
                    name: t("graph.stormguard.actions"),
                    type: "scatter",
                    data: markers,
                    xAxisIndex: 0,
                    yAxisIndex: 0,
                    symbol: "diamond",
                    symbolSize: 12,
                    tooltip: {formatter: (item) => item.data.name},
                },
                line(t("graph.stormguard.effective_rtt"), seriesData(points, "effectiveRttMs"), 1, 1, palette[0]),
                line(t("graph.stormguard.passive_rtt"), seriesData(points, "passiveRttMs"), 1, 1, palette[1]),
                line(t("graph.stormguard.active_rtt"), seriesData(points, "activeRttMs"), 1, 1, palette[3]),
                line(t("graph.stormguard.baseline"), seriesData(points, "baselineRttMs"), 1, 1, palette[2], {lineStyle: {type: "dashed"}}),
                line(t("graph.stormguard.delay"), seriesData(points, "delayMs"), 1, 1, palette[4]),
                line(t("graph.stormguard.score"), seriesData(points, "decisionScore"), 2, 2, palette[0]),
                line(t("graph.stormguard.cooldown"), seriesData(points, "cooldownSeconds"), 2, 3, palette[3]),
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
