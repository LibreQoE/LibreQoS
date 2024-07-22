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
            } else {
                document.body.classList.remove('dark-mode');
                document.documentElement.setAttribute('data-bs-theme', "light");
                localStorage.setItem('theme', 'light');
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
            }
            if (window.graphList !== undefined) {
                window.graphList.forEach((graph) => {
                    graph.chart.dispose();
                    if (darkModeSwitch.checked) {
                        graph.chart = echarts.init(graph.dom, 'dark');
                    } else {
                        graph.chart = echarts.init(graph.dom, 'vintage');
                    }
                    graph.chart.setOption(graph.option);
                });
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