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
    cssRedact();
}

export function redactCell(cell) {
    cell.classList.add("redactable");
}

function cssRedact() {
    if (isRedacted()) {
        let css = css_getclass(".redactable");
        css.style.filter = "blur(4px)";
    } else {
        let css = css_getclass(".redactable");
        css.style.filter = "";
    }
}

function isRedacted() {
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

function cssrules() {
    var rules = {};
    for (var i = 0; i < document.styleSheets.length; ++i) {
        var cssRules = document.styleSheets[i].cssRules;
        for (var j = 0; j < cssRules.length; ++j)
            rules[cssRules[j].selectorText] = cssRules[j];
    }
    return rules;
}

function css_getclass(name) {
    var rules = cssrules();
    if (!rules.hasOwnProperty(name))
        throw 'TODO: deal_with_notfound_case';
    return rules[name];
}