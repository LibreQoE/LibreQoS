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