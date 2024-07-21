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
            } else {
                document.body.classList.remove('dark-mode');
                document.documentElement.setAttribute('data-bs-theme', "light");
                localStorage.setItem('theme', 'light');
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