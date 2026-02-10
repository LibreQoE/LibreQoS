import {DashboardGraph} from "./dashboard_graph";
import {colorByQoqScore} from "../helpers/color_scales";

export class QooScoreGauge extends DashboardGraph {
    constructor(id) {
        super(id);

        this.gradient = new echarts.graphic.LinearGradient(
            0, 1, 0, 0,
            [
                { offset: 0.0, color: '#ff0000' },
                { offset: 0.5, color: '#ffa500' },
                { offset: 1.0, color: '#00ff00' },
            ]
        );

        this.option = {
            grid: {
                left: 26,
                right: 16,
                top: 18,
                bottom: 26,
            },
            xAxis: {
                type: 'category',
                data: ['QoO'],
                axisLine: { show: false },
                axisTick: { show: false },
                axisLabel: { show: false },
            },
            yAxis: {
                type: 'value',
                min: 0,
                max: 100,
                splitNumber: 4,
                axisLabel: { fontSize: 10, color: '#aaa' },
                axisTick: { show: false },
                splitLine: { show: false },
            },
            series: [
                // Background (full 0..100 scale)
                {
                    type: 'bar',
                    data: [100],
                    barWidth: 18,
                    silent: true,
                    itemStyle: { color: this.gradient, opacity: 0.25 },
                    z: 1,
                },
                // Marker (current score)
                {
                    type: 'scatter',
                    data: [],
                    symbol: 'rect',
                    symbolSize: [22, 6],
                    itemStyle: { color: '#fff' },
                    z: 3,
                },
            ],
            graphic: [
                {
                    type: 'text',
                    left: 'center',
                    top: 0,
                    style: {
                        text: 'QoO',
                        fill: '#aaa',
                        fontSize: 11,
                        fontWeight: 'bold',
                        align: 'center',
                    },
                },
                {
                    type: 'text',
                    left: 'center',
                    bottom: 0,
                    style: {
                        text: '—',
                        fill: '#aaa',
                        fontSize: 12,
                        fontWeight: 'bold',
                        align: 'center',
                    },
                },
            ],
        };

        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    update(score0to100) {
        this.chart.hideLoading();

        if (score0to100 === null || score0to100 === undefined) {
            this.option.series[1].data = [];
            this.option.graphic[1].style.text = '—';
            this.option.graphic[1].style.fill = '#aaa';
            this.chart.setOption(this.option);
            return;
        }

        const raw = Number(score0to100);
        if (!Number.isFinite(raw) || raw === 255) {
            this.option.series[1].data = [];
            this.option.graphic[1].style.text = '—';
            this.option.graphic[1].style.fill = '#aaa';
            this.chart.setOption(this.option);
            return;
        }

        const clamped = Math.min(100, Math.max(0, raw));
        this.option.series[1].data = [clamped];
        this.option.series[1].itemStyle = { color: colorByQoqScore(clamped) };
        this.option.graphic[1].style.text = Math.round(clamped).toString();
        this.option.graphic[1].style.fill = colorByQoqScore(clamped);
        this.chart.setOption(this.option);
    }
}

