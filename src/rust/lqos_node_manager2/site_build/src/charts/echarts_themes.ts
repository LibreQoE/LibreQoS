import { registerTheme } from "echarts";
import * as echarts from 'echarts';

var themesRegistered = false;

export function registerThemes() {
    if (!themesRegistered) {
        registerInfographicTheme();
        registerDarkTheme();
        themesRegistered = true;
    }
}

export function initEchartsWithTheme(div: HTMLDivElement): echarts.ECharts {
    let storedTheme = localStorage.getItem('theme');
    switch (storedTheme) {
        case 'dark':
            let dark = echarts.init(div, 'dark');
            dark.group = 'echarts';
            echarts.connect('echarts');
            return dark;
        default:
            let light = echarts.init(div, 'default');
            light.group = 'echarts';
            echarts.connect('echarts');
            return light;
    }
}

function registerInfographicTheme() {
    var colorPalette = [
        '#C1232B',
        '#27727B',
        '#FCCE10',
        '#E87C25',
        '#B5C334',
        '#FE8463',
        '#9BCA63',
        '#FAD860',
        '#F3A43B',
        '#60C0DD',
        '#D7504B',
        '#C6E579',
        '#F4E001',
        '#F0805A',
        '#26C0C0'
    ];

    var theme = {
        color: colorPalette,

        title: {
            textStyle: {
                fontWeight: 'normal',
                color: '#27727B'
            }
        },

        visualMap: {
            color: ['#C1232B', '#FCCE10']
        },

        toolbox: {
            iconStyle: {
                borderColor: colorPalette[0]
            }
        },

        tooltip: {
            backgroundColor: 'rgba(50,50,50,0.5)',
            axisPointer: {
                type: 'line',
                lineStyle: {
                    color: '#ff9999',
                    type: 'dashed'
                },
                crossStyle: {
                    color: '#27727B'
                },
                shadowStyle: {
                    color: 'rgba(200,200,200,0.3)'
                }
            }
        },

        dataZoom: {
            dataBackgroundColor: 'rgba(181,195,52,0.3)',
            fillerColor: 'rgba(181,195,52,0.2)',
            handleColor: '#27727B'
        },

        categoryAxis: {
            axisLine: {
                lineStyle: {
                    color: '#27727B'
                }
            },
            splitLine: {
                show: false
            }
        },

        valueAxis: {
            axisLine: {
                show: false
            },
            splitArea: {
                show: false
            },
            splitLine: {
                lineStyle: {
                    color: ['#ccc'],
                    type: 'dashed'
                }
            }
        },

        timeline: {
            itemStyle: {
                color: '#27727B'
            },
            lineStyle: {
                color: '#27727B'
            },
            controlStyle: {
                color: '#27727B',
                borderColor: '#27727B'
            },
            symbol: 'emptyCircle',
            symbolSize: 3
        },

        line: {
            itemStyle: {
                borderWidth: 2,
                borderColor: '#fff',
                lineStyle: {
                    width: 3
                }
            },
            emphasis: {
                itemStyle: {
                    borderWidth: 0
                }
            },
            symbol: 'circle',
            symbolSize: 3.5
        },

        candlestick: {
            itemStyle: {
                color: '#c1232b',
                color0: '#b5c334'
            },
            lineStyle: {
                width: 1,
                color: '#c1232b',
                color0: '#b5c334'
            },
            areaStyle: {
                color: '#c1232b',
                color0: '#27727b'
            }
        },

        graph: {
            itemStyle: {
                color: '#c1232b'
            },
            linkStyle: {
                color: '#b5c334'
            }
        },

        map: {
            itemStyle: {
                color: '#f2385a',
                areaColor: '#ddd',
                borderColor: '#eee'
            },
            areaStyle: {
                color: '#fe994e'
            },
            label: {
                color: '#c1232b'
            }
        },

        gauge: {
            axisLine: {
                lineStyle: {
                    color: [
                        [0.2, '#B5C334'],
                        [0.8, '#27727B'],
                        [1, '#C1232B']
                    ]
                }
            },
            axisTick: {
                splitNumber: 2,
                length: 5,
                lineStyle: {
                    color: '#fff'
                }
            },
            axisLabel: {
                color: '#fff'
            },
            splitLine: {
                length: '5%',
                lineStyle: {
                    color: '#fff'
                }
            },
            title: {
                offsetCenter: [0, -20]
            }
        }
    };

    registerTheme('infographic', theme);
}

function registerDarkTheme() {
    var contrastColor = '#B9B8CE';
    var backgroundColor = '#100C2A';
    var axisCommon = function () {
        return {
            axisLine: {
                lineStyle: {
                    color: contrastColor
                }
            },
            splitLine: {
                lineStyle: {
                    color: '#484753'
                }
            },
            splitArea: {
                areaStyle: {
                    color: ['rgba(255,255,255,0.02)', 'rgba(255,255,255,0.05)']
                }
            },
            minorSplitLine: {
                lineStyle: {
                    color: '#20203B'
                }
            }
        };
    };

    var colorPalette = [
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
    var theme = {
        darkMode: true,

        color: colorPalette,
        //backgroundColor: backgroundColor,
        axisPointer: {
            lineStyle: {
                color: '#ff9999'
            },
            crossStyle: {
                color: '#817f91'
            },
            label: {
                // TODO Contrast of label backgorundColor
                color: '#fff'
            }
        },
        legend: {
            textStyle: {
                color: contrastColor
            }
        },
        textStyle: {
            color: contrastColor
        },
        title: {
            textStyle: {
                color: '#EEF1FA'
            },
            subtextStyle: {
                color: '#B9B8CE'
            }
        },
        toolbox: {
            iconStyle: {
                borderColor: contrastColor
            }
        },
        dataZoom: {
            borderColor: '#71708A',
            textStyle: {
                color: contrastColor
            },
            brushStyle: {
                color: 'rgba(135,163,206,0.3)'
            },
            handleStyle: {
                color: '#353450',
                borderColor: '#C5CBE3'
            },
            moveHandleStyle: {
                color: '#B0B6C3',
                opacity: 0.3
            },
            fillerColor: 'rgba(135,163,206,0.2)',
            emphasis: {
                handleStyle: {
                    borderColor: '#91B7F2',
                    color: '#4D587D'
                },
                moveHandleStyle: {
                    color: '#636D9A',
                    opacity: 0.7
                }
            },
            dataBackground: {
                lineStyle: {
                    color: '#71708A',
                    width: 1
                },
                areaStyle: {
                    color: '#71708A'
                }
            },
            selectedDataBackground: {
                lineStyle: {
                    color: '#87A3CE'
                },
                areaStyle: {
                    color: '#87A3CE'
                }
            }
        },
        visualMap: {
            textStyle: {
                color: contrastColor
            }
        },
        timeline: {
            lineStyle: {
                color: contrastColor
            },
            label: {
                color: contrastColor
            },
            controlStyle: {
                color: contrastColor,
                borderColor: contrastColor
            }
        },
        calendar: {
            itemStyle: {
                color: backgroundColor
            },
            dayLabel: {
                color: contrastColor
            },
            monthLabel: {
                color: contrastColor
            },
            yearLabel: {
                color: contrastColor
            }
        },
        timeAxis: axisCommon(),
        logAxis: axisCommon(),
        valueAxis: axisCommon(),
        categoryAxis: axisCommon(),

        line: {
            symbol: 'circle'
        },
        graph: {
            color: colorPalette
        },
        gauge: {
            title: {
                color: contrastColor
            }
        },
        candlestick: {
            itemStyle: {
                color: '#FD1050',
                color0: '#0CF49B',
                borderColor: '#FD1050',
                borderColor0: '#0CF49B'
            }
        }
    };

    theme.categoryAxis.splitLine.show = false;
    registerTheme('dark', theme);
}