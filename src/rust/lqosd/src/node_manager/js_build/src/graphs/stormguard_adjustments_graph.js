import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

function formatTime(ts) {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-US', { hour12: false });
}

export class StormguardAdjustmentsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new StormguardRingBuffer(RING_SIZE);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Bandwidth Adjustments", 50)
            .build();

        // Custom Y-axis to show both positive and negative
        this.option.yAxis = {
            type: 'value',
            name: 'Bandwidth Adjustments',
            nameLocation: 'middle',
            nameGap: 50,
            axisLabel: {
                formatter: (val) => {
                    if (val === 0) return '0';
                    return (val > 0 ? '+' : '') + scaleNumber(val, 0);
                },
            },
            splitLine: {
                lineStyle: {
                    color: '#333'
                }
            }
        };

        // Add a zero line
        this.option.series = [
            {
                name: 'Zero Line',
                type: 'line',
                data: Array(RING_SIZE).fill(0),
                lineStyle: {
                    color: '#666',
                    width: 1,
                    type: 'dashed'
                },
                symbol: 'none',
                silent: true,
                animation: false,
                z: 1
            },
            {
                name: 'Increases',
                data: [],
                type: 'bar',
                barWidth: '60%',
                itemStyle: {
                    color: window.graphPalette[0],
                },
                animationDuration: 300,
                animationEasing: 'cubicOut'
            },
            {
                name: 'Decreases',
                data: [],
                type: 'bar',
                barWidth: '60%',
                itemStyle: {
                    color: window.graphPalette[3],
                },
                animationDuration: 300,
                animationEasing: 'cubicOut'
            },
        ];

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [
                {
                    name: "Bandwidth Increases",
                    icon: 'rect',
                    itemStyle: {
                        color: window.graphPalette[0]
                    }
                },
                {
                    name: "Bandwidth Decreases", 
                    icon: 'rect',
                    itemStyle: {
                        color: window.graphPalette[3]
                    }
                }
            ],
            textStyle: {
                color: '#aaa'
            },
        };

        // Add axisPointer and tooltip with time display
        this.option.tooltip = {
            trigger: 'axis',
            axisPointer: {
                type: 'shadow',
                label: {
                    backgroundColor: '#6a7985'
                }
            },
            formatter: (params) => {
                if (!params || params.length === 0) return '';
                const idx = params[0].dataIndex;
                const ts = this.ringbuffer.getTimestamp(idx);
                const data = this.ringbuffer.getDataAt(idx);
                
                let s = `<div><b>Time:</b> ${formatTime(ts)}</div>`;
                s += `<div><b>Sites Evaluated:</b> ${data.evaluated}</div>`;
                
                if (data.adjustmentsUp > 0) {
                    s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${window.graphPalette[0]};"></span>Increases: <b>+${data.adjustmentsUp}</b></div>`;
                }
                if (data.adjustmentsDown > 0) {
                    s += `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${window.graphPalette[3]};"></span>Decreases: <b>-${data.adjustmentsDown}</b></div>`;
                }
                
                const netChange = data.adjustmentsUp - data.adjustmentsDown;
                s += `<div><b>Net Change:</b> ${netChange > 0 ? '+' : ''}${netChange}</div>`;
                
                return s;
            }
        };

        // Animation on data change
        this.option.animation = true;
        this.option.animationDuration = 500;
        this.option.animationEasing = 'elasticOut';

        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.series[1].itemStyle.color = window.graphPalette[0];
        this.option.series[2].itemStyle.color = window.graphPalette[3];
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[3];
        
        this.chart.setOption(this.option);
    }

    update(adjustmentsUp, adjustmentsDown, sitesEvaluated) {
        this.chart.hideLoading();
        this.ringbuffer.push(adjustmentsUp, adjustmentsDown, sitesEvaluated, Date.now());

        let data = this.ringbuffer.series();
        
        // Update bar data
        this.option.series[1].data = data.increases;
        this.option.series[2].data = data.decreases;

        // Add animation emphasis on new data
        if (adjustmentsUp > 0 || adjustmentsDown > 0) {
            this.option.series[1].markPoint = {
                animation: true,
                animationDuration: 1000,
                animationEasing: 'bounceOut',
                data: adjustmentsUp > 0 ? [
                    {
                        coord: [RING_SIZE - 1, adjustmentsUp],
                        symbol: 'arrow',
                        symbolSize: 20,
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }
                ] : []
            };
            this.option.series[2].markPoint = {
                animation: true,
                animationDuration: 1000,
                animationEasing: 'bounceOut',
                data: adjustmentsDown > 0 ? [
                    {
                        coord: [RING_SIZE - 1, -adjustmentsDown],
                        symbol: 'arrow',
                        symbolSize: 20,
                        symbolRotate: 180,
                        itemStyle: {
                            color: window.graphPalette[3]
                        }
                    }
                ] : []
            };
        }

        this.chart.setOption(this.option);
    }
}

class StormguardRingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push({
                adjustmentsUp: 0,
                adjustmentsDown: 0,
                evaluated: 0,
                timestamp: 0
            });
        }
        this.head = 0;
        this.data = data;
    }

    push(adjustmentsUp, adjustmentsDown, evaluated, timestamp) {
        this.data[this.head] = {
            adjustmentsUp: adjustmentsUp || 0,
            adjustmentsDown: adjustmentsDown || 0,
            evaluated: evaluated || 0,
            timestamp: timestamp || Date.now()
        };
        this.head += 1;
        this.head %= this.size;
    }

    getTimestamp(idx) {
        let physical = (this.head + idx) % this.size;
        return this.data[physical].timestamp;
    }

    getDataAt(idx) {
        let physical = (this.head + idx) % this.size;
        return this.data[physical];
    }

    series() {
        let increases = [];
        let decreases = [];
        
        for (let i=this.head; i<this.size; i++) {
            increases.push(this.data[i].adjustmentsUp);
            decreases.push(-this.data[i].adjustmentsDown); // Negative for display
        }
        for (let i=0; i<this.head; i++) {
            increases.push(this.data[i].adjustmentsUp);
            decreases.push(-this.data[i].adjustmentsDown); // Negative for display
        }
        
        return { increases, decreases };
    }
}