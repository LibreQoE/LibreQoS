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
        //console.log(data);
        $("#shapedDeviceCount").text(data.shaped_devices);
        $("#unknownIpCount").text(data.unknown_ips);
    })
}

function initLogout() {
    $("#btnLogout").on('click', () => {
        //console.log("Logout");
        const cookies = document.cookie.split(";");

        for (let i = 0; i < cookies.length; i++) {
            const cookie = cookies[i];
            const eqPos = cookie.indexOf("=");
            const name = eqPos > -1 ? cookie.substr(0, eqPos) : cookie;
            document.cookie = name + "=;expires=Thu, 01 Jan 1970 00:00:00 GMT";
        }
        window.location.reload();
    });
}

function titleAndLts() {
    $.get("/local-api/ltsCheck", (data) => {
        // Set the title
        if (data.node_name !== null) {
            document.title = data.node_name + " - LibreQoS Node Manager";
        }
    });
}

initLogout();
initDayNightMode();
getDeviceCounts();
titleAndLts();