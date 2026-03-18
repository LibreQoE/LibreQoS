function getSavedTheme() {
    const currentTheme = localStorage.getItem('theme');
    if (currentTheme === 'dark' || currentTheme === 'light') {
        return currentTheme;
    }
    return null;
}

function getEffectiveTheme() {
    return getSavedTheme() || 'dark';
}

function setGraphPalette(theme) {
    if (theme === 'dark') {
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
        return;
    }

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

function applyTheme(theme, darkModeSwitch) {
    const isDark = theme === 'dark';

    if (darkModeSwitch) {
        darkModeSwitch.checked = isDark;
    }

    document.body.classList.toggle('dark-mode', isDark);
    document.documentElement.setAttribute('data-bs-theme', isDark ? 'dark' : 'light');
    setGraphPalette(theme);
}

function reinitializeGraphs(theme) {
    if (window.graphList === undefined) {
        return;
    }

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
        graph.chart = echarts.init(graph.dom, theme === 'dark' ? 'dark' : 'vintage');
        graph.chart.setOption(graph.option);
        if (graph.onThemeChange) {
            graph.onThemeChange();
        }
        next.push(graph);
    });
    window.graphList = next;
}

export function initDayNightMode() {
    applyTheme(getEffectiveTheme(), document.getElementById('darkModeSwitch'));

    document.addEventListener('DOMContentLoaded', () => {
        const darkModeSwitch = document.getElementById('darkModeSwitch');
        applyTheme(getEffectiveTheme(), darkModeSwitch);

        darkModeSwitch.addEventListener('change', () => {
            const theme = darkModeSwitch.checked ? 'dark' : 'light';
            localStorage.setItem('theme', theme);
            applyTheme(theme, darkModeSwitch);
            reinitializeGraphs(theme);
        });
    });
}

export function isDarkMode() {
    return getEffectiveTheme() === 'dark';
}
