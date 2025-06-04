export function initRedact() {
    let modeSwitch = document.getElementById("redactSwitch");
    modeSwitch.checked = isRedacted();
    modeSwitch.onclick = () => {
        let modeSwitch = document.getElementById("redactSwitch");
        if (modeSwitch.checked) {
            localStorage.setItem("redact", "true");
        } else {
            localStorage.setItem("redact", "false");
        }
        cssRedact();
    };

    let klingonSwitch = document.getElementById("klingonSwitch");
    if (klingonSwitch !== null) {
        klingonSwitch.checked = localStorage.getItem("klingon") === "true";
        klingonSwitch.onclick = () => {
            let klingonSwitch = document.getElementById("klingonSwitch");
            if (klingonSwitch.checked) {
                localStorage.setItem("klingon", "true");
            } else {
                localStorage.setItem("klingon", "false");
            }
            cssRedact();
        };
    }

    cssRedact();
}

export function redactCell(cell) {
    cell.classList.add("redactable");
}

function cssRedact() {
    let r = document.querySelector(':root');
    if (isRedacted()) {
        r.style.setProperty('--redact', 'blur(8px)');
    } else {
        r.style.setProperty('--redact', 'none');
    }

    if (isKlingon()) {
        r.style.setProperty('--redact-font-family', '"Klingon", sans-serif');
    } else {
        r.style.setProperty('--redact-font-family', 'inherit');
    }
}

export function isRedacted() {
    let prefs = localStorage.getItem("redact");
    if (prefs === null) {
        localStorage.setItem("redact", "false");
        return false;
    }
    if (prefs === "false") {
        return false;
    }
    if (prefs === "true") {
        return true;
    }
}

export function isKlingon() {
    let prefs = localStorage.getItem("klingon");
    if (prefs === null) {
        localStorage.setItem("klingon", "false");
        return false;
    }
    if (prefs === "false") {
        return false;
    }
    if (prefs === "true") {
        return true;
    }
}