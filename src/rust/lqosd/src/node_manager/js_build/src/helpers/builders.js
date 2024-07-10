export function heading5Icon(icon, text) {
    let h5 = document.createElement("h5");
    h5.innerHTML = "<i class='fa fa-" + icon + "'></i> " + text;
    return h5;
}

export function theading(text, colspan=0, tooltip="", id="") {
    let th = document.createElement("th");
    if (id !== "") th.id = id;
    if (colspan > 0) th.colSpan = colspan;

    if (tooltip !== "") {
        th.setAttribute("data-bs-toggle", "tooltip");
        th.setAttribute("data-bs-placement", "top");
        th.setAttribute("data-bs-html", "true");
        th.setAttribute("title", tooltip);
        th.innerHTML = text + " <i class='fas fa-info-circle'></i>";
    } else {
        th.innerText = text;
    }

    return th;
}

export function simpleRow(text) {
    let td = document.createElement("td");
    td.innerText = text;
    return td;
}

export function simpleRowHtml(text) {
    let td = document.createElement("td");
    td.innerHTML = text;
    return td;
}

export function clearDashDiv(id, target) {
    let limit = 1;
    if (id.includes("___")) limit = 0;
    while (target.children.length > limit) {
        target.removeChild(target.lastChild);
    }
}

export function clearDiv(target, targetLength=1) {
    while (target.children.length > targetLength) {
        target.removeChild(target.lastChild);
    }
}

export function enableTooltips() {
    // Tooltips everywhere!
    let tooltipTriggerList = [].slice.call(document.querySelectorAll('[data-bs-toggle="tooltip"]'))
    let tooltipList = tooltipTriggerList.map(function (tooltipTriggerEl) {
        return new bootstrap.Tooltip(tooltipTriggerEl)
    })
}

let pendingTooltips = [];

export function tooltipsNextFrame(id) {
    pendingTooltips.push(id);
    requestAnimationFrame(() => {
        setTimeout(() => {
            pendingTooltips.forEach((id) => {
                let tooltipTriggerEl = document.getElementById(id);
                if (tooltipTriggerEl !== null) {
                    new bootstrap.Tooltip(tooltipTriggerEl);
                }
            });
            pendingTooltips = [];
        })
    });
}