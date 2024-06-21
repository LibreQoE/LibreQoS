// Setup any WS feeds for this page
let ws = null;

function subscribeWS(channels, handler) {
    if (ws) {
        ws.close();
    }

    ws = new WebSocket('ws://' + window.location.host + '/ws');
    ws.onopen = () => {
        for (let i=0; i<channels.length; i++) {
            ws.send("{ \"channel\" : \"" + channels[i] + "\"}");
        }
    }
    ws.onclose = () => {
        ws = null;
    }
    ws.onerror = (error) => {
        ws = null
    }
    ws.onmessage = function (event) {
        let msg = JSON.parse(event.data);
        handler(msg);
    };
}



// Fires on start for all pages. Called by the template.
function pageInit() {
    InitDayNightMode();
}

// Initializes day/night mode. Called by the template.
function InitDayNightMode() {
    const currentTheme = localStorage.getItem('theme');
    if (currentTheme === 'dark') {
        const darkModeSwitch = document.getElementById('darkModeSwitch');
        darkModeSwitch.checked = true;
        document.body.classList.add('dark-mode');
        document.documentElement.setAttribute('data-bs-theme', "dark");
    } else {
        document.body.classList.remove('dark-mode');
        document.documentElement.setAttribute('data-bs-theme', "light");
    }

    document.addEventListener('DOMContentLoaded', (event) => {
        const darkModeSwitch = document.getElementById('darkModeSwitch');
        const currentTheme = localStorage.getItem('theme');

        if (currentTheme === 'dark') {
            document.body.classList.add('dark-mode');
            darkModeSwitch.checked = true;
        }

        darkModeSwitch.addEventListener('change', () => {
            if (darkModeSwitch.checked) {
                document.body.classList.add('dark-mode');
                document.documentElement.setAttribute('data-bs-theme', "dark");
                localStorage.setItem('theme', 'dark');
            } else {
                document.body.classList.remove('dark-mode');
                document.documentElement.setAttribute('data-bs-theme', "light");
                localStorage.setItem('theme', 'light');
            }

            if (window.router) {
                window.router.onThemeSwitch();
            }
        });
    });
}

class DashboardGraph {
    constructor(id) {
        this.id = id;
        this.dom = document.getElementById(id);
        this.chart = echarts.init(this.dom);
        this.chart.showLoading();
        this.option = {};
    }
}

class DashboardGauge extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
              {
                type: 'gauge',
                axisLine: {
                  lineStyle: {
                    width: 10,
                    color: [
                      [0.5, 'green'],
                      [0.8, 'orange'],
                      [1, '#fd666d']
                    ]
                  }
                },
                pointer: {
                  itemStyle: {
                    color: 'auto'
                  }
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
                  length: 15,
                  lineStyle: {
                    color: '#999',
                    width: 4
                  }
                },
                axisLabel: {
                  color: 'inherit',
                  distance: 16,
                  fontSize: 10,
                  formatter: (value) => { return scaleNumber(value, 1); }
                },
                detail: {
                    valueAnimation: true,
                    formatter: (value) => { return scaleNumber(value); },
                    color: 'inherit',
                    fontSize: 12,
                },
                title: {
                    fontSize: 14,
                    color: 'inherit',
                },
                data: [
                  {
                    name: "UP",
                    value: 0,
                    title: { offsetCenter: ['-40%', '75%'] },
                    detail: { offsetCenter: ['-40%', '95%'] },
                  },
                  {
                    name: "DOWN",
                    value: 0,
                    title: { offsetCenter: ['40%', '75%'] },
                    detail: { offsetCenter: ['40%', '95%'] },
                  }
                ]
              }
            ]
          };
        this.option && this.chart.setOption(this.option);
    }

    update(value1, value2, max1, max2) {
        this.chart.hideLoading();
        this.option.series[0].data[0].value = value1;
        this.option.series[0].data[1].value = value2;
        this.option.series[0].min = 0;
        this.option.series[0].max = Math.max(max1, max2) * 1000000; // Convert to bits
        this.chart.setOption(this.option);
    }
}

class PacketsBar extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            xAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (value) => { return scaleNumber(value, 0); }
                }
            },
            yAxis: {
                type: 'category',
                data: ['Up', 'Down'],
            },
            series: [
                {
                    type: 'bar',
                    data: [0, 0]
                }
            ]
        }
        this.option && this.chart.setOption(this.option);
    }

    update(up, down) {
        this.chart.hideLoading();
        this.option.series[0].data = [up, down];
        this.chart.setOption(this.option);
    }
}

function scaleNumber(n, fixed=2) {
    if (n > 1000000000000) {
        return (n / 1000000000000).toFixed(fixed) + "T";
    } else if (n > 1000000000) {
        return (n / 1000000000).toFixed(fixed) + "G";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(fixed) + "M";
    } else if (n > 1000) {
        return (n / 1000).toFixed(fixed) + "K";
    }
    return n;
}