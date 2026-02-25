export function initDayNightMode() {
    if (isDarkMode()) {
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
                document.body.classList.remove('dark-mode');
                document.documentElement.setAttribute('data-bs-theme', "light");
                localStorage.setItem('theme', 'light');
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
            if (window.graphList !== undefined) {
                const next = [];
                window.graphList.forEach((graph) => {
                    if (!graph || !graph.dom) return;
                    if (typeof graph.dom.isConnected === "boolean" && !graph.dom.isConnected) {
                        // Drop charts whose DOM has been removed (e.g. closed zoom overlay).
                        try {
                            if (graph.chart && graph.chart.dispose) {
                                graph.chart.dispose();
                            }
                        } catch (_) {}
                        return;
                    }

                    // Defensive: dispose any existing instance before re-init.
                    try {
                        if (graph.chart && graph.chart.dispose) {
                            graph.chart.dispose();
                        } else if (typeof echarts !== "undefined" && echarts.getInstanceByDom) {
                            const existing = echarts.getInstanceByDom(graph.dom);
                            if (existing) existing.dispose();
                        }
                    } catch (_) {}

                    if (typeof echarts === "undefined" || !echarts.init) {
                        return;
                    }
                    graph.chart = echarts.init(graph.dom, darkModeSwitch.checked ? 'dark' : 'vintage');
                    graph.chart.setOption(graph.option);
                    if (graph.onThemeChange) {
                        graph.onThemeChange();
                    }
                    next.push(graph);
                });
                window.graphList = next;
            }
        });
    });
}

export function isDarkMode() {
    const currentTheme = localStorage.getItem('theme');
    if (currentTheme === null) {
        if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
            localStorage.setItem('theme', 'dark');
            return true;
        } else {
            localStorage.setItem('theme', 'light');
            return false;
        }
    }
    return currentTheme === 'dark';
}
