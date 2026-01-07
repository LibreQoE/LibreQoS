import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

export class BitsPerSecondGauge extends DashboardGraph {
    constructor(id, thresholdLabel = 'Plan') {
        super(id);
        this.thresholdLabel = thresholdLabel;
        this.option = {
            graphic: [
                {
                    type: 'text',
                    left: 'center',
                    top: '4%',
                    style: {
                        text: this.thresholdLabel,
                        fill: '#aaa',
                        fontSize: 11,
                        fontWeight: 'bold',
                        align: 'center'
                    }
                }
            ],
            series: [
                {
                    type: 'gauge',
                    splitNumber: 2,
                    z: 3, // Render Download above Upload
                    axisLine: {
                        lineStyle: {
                            width: 10,
                            color: [
                                [0.5, 'green'],
                                [0.75, 'orange'],
                                [1, '#fd666d']
                            ]
                        }
                    },
                    pointer: {
                        icon: 'path://M2.9,0.7L2.9,0.7c1.4,0,2.6,1.2,2.6,2.6v115c0,1.4-1.2,2.6-2.6,2.6l0,0c-1.4,0-2.6-1.2-2.6-2.6V3.3C0.3,1.9,1.4,0.7,2.9,0.7z',
                        itemStyle: {
                            // Download color = blue (palette[0])
                            color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[0] : '#4992ff'
                        },
                        width: 4,
                        length: '85%'
                    },
                    axisTick: {
                        distance: -10,
                        length: 8,
                        lineStyle: {
                            color: '#fff',
                            width: 2
                        }
                    },
                    splitLine: {
                        distance: -15,
                        length: 18,
                        lineStyle: {
                            color: '#aaa',
                            width: 4
                        }
                    },
                    axisLabel: {
                        show: false
                    },
                    detail: {
                        valueAnimation: true,
                        formatter: (value) => { return scaleNumber(value); },
                        // Download number color = blue
                        color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[0] : '#4992ff',
                        fontSize: 12,
                    },
                    title: {
                        fontSize: 12,
                        // Download label color = blue
                        color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[0] : '#4992ff',
                    },
                    data: [
                        {
                            name: "DOWN",
                            value: 0,
                            // Place DOWN on the left side for consistency; shift slightly lower
                            title: { offsetCenter: ['-45%', '88%'] },
                            detail: { offsetCenter: ['-45%', '102%'] },
                        }
                    ]
                },
                {
                    type: 'gauge',
                    splitNumber: 2,
                    z: 2,
                    axisLine: {
                        lineStyle: {
                            width: 10,
                            color: [
                                [0.5, 'green'],
                                [0.75, 'orange'],
                                [1, '#fd666d']
                            ]
                        }
                    },
                    pointer: {
                        itemStyle: {
                            // Upload color = green (palette[1])
                            color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[1] : '#7cffb2',
                        },
                        length: '85%',
                        width: 4,
                        icon: 'path://M2.9,0.7L2.9,0.7c1.4,0,2.6,1.2,2.6,2.6v115c0,1.4-1.2,2.6-2.6,2.6l0,0c-1.4,0-2.6-1.2-2.6-2.6V3.3C0.3,1.9,1.4,0.7,2.9,0.7z',
                    },
                    axisTick: {
                        distance: -10,
                        length: 8,
                        lineStyle: {
                            color: '#fff',
                            width: 2
                        }
                    },
                    splitLine: {
                        distance: -15,
                        length: 18,
                        lineStyle: {
                            color: '#aaa',
                            width: 4
                        }
                    },
                    axisLabel: {
                        show: false
                    },
                    detail: {
                        valueAnimation: true,
                        formatter: (value) => { return scaleNumber(value); },
                        // Upload number color = green
                        color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[1] : '#7cffb2',
                        fontSize: 12,
                    },
                    title: {
                        fontSize: 12,
                        // Upload label color = green
                        color: (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[1] : '#7cffb2',
                    },
                    data: [
                        {
                            name: "UP",
                            value: 0,
                            // Place UP on the right side for consistency; shift slightly lower
                            title: { offsetCenter: ['45%', '88%'] },
                            detail: { offsetCenter: ['45%', '102%'] },
                        },
                    ]
                },
            ]
        };
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        // Re-apply palette-based colors when theme toggles
        try {
            const blue = (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[0] : '#4992ff';
            const green = (typeof window !== 'undefined' && window.graphPalette) ? window.graphPalette[1] : '#7cffb2';
            const labelColor = (typeof document !== 'undefined' && document.documentElement.getAttribute('data-bs-theme') === 'dark') ? '#aaa' : '#666';

            // Download (series 0)
            if (this.option.series[0].pointer && this.option.series[0].pointer.itemStyle) {
                this.option.series[0].pointer.itemStyle.color = blue;
            }
            if (this.option.series[0].detail) {
                this.option.series[0].detail.color = blue;
            }
            if (this.option.series[0].title) {
                this.option.series[0].title.color = blue;
            }

            // Upload (series 1)
            if (this.option.series[1].pointer && this.option.series[1].pointer.itemStyle) {
                this.option.series[1].pointer.itemStyle.color = green;
            }
            if (this.option.series[1].detail) {
                this.option.series[1].detail.color = green;
            }
            if (this.option.series[1].title) {
                this.option.series[1].title.color = green;
            }

            // Update threshold label styling/text
            if (!this.option.graphic) this.option.graphic = [];
            if (this.option.graphic.length > 0 && this.option.graphic[0].type === 'text') {
                this.option.graphic[0].style.fill = labelColor;
                this.option.graphic[0].style.text = this.thresholdLabel || 'Plan';
            }

            this.chart.setOption(this.option);
        } catch (_) {
            // No-op on any unexpected structure
        }
    }

    update(download, upload, max_down, max_up) {
        this.chart.hideLoading();
        download = toNumber(download, 0);
        upload = toNumber(upload, 0);
        max_down = toNumber(max_down, 0);
        max_up = toNumber(max_up, 0);

        this.option.series[0].data[0].value = download;
        this.option.series[1].data[0].value = upload;
        this.option.series[0].min = 0;
        this.option.series[0].max = (max_down * 2) * 1000000; // 2x plan speed
        this.option.series[1].min = 0;
        this.option.series[1].max = (max_up * 2) * 1000000; // 2x plan speed
        this.chart.setOption(this.option);
    }
}
