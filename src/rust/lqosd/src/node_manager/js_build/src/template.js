function initDayNightMode() {
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

function getDeviceCounts() {
    $.get("/local-api/deviceCount", (data) => {
        console.log(data);
        $("#shapedDeviceCount").text(data.shaped_devices);
        $("#unknownIpCount").text(data.unknown_ips);
    })
}

initDayNightMode();
getDeviceCounts();