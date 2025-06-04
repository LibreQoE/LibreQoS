export function initColorBlind() {
    let modeSwitch = document.getElementById("colorBlindSwitch");
    modeSwitch.checked = isColorBlindMode();
    modeSwitch.onclick = () => {
        let modeSwitch = document.getElementById("colorBlindSwitch");
        if (modeSwitch.checked) {
            localStorage.setItem("colorBlindMode", "true");
        } else {
            localStorage.setItem("colorBlindMode", "false");
        }
        // Trigger a custom event that components can listen to
        window.dispatchEvent(new Event('colorBlindModeChanged'));
    };
}

export function isColorBlindMode() {
    let prefs = localStorage.getItem("colorBlindMode");
    if (prefs === null) {
        localStorage.setItem("colorBlindMode", "false");
        return false;
    }
    return prefs === "true";
}