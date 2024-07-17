import {formatRetransmit, formatRtt, formatThroughput, scaleNanos} from "./scaling";
import {redactCell} from "./redact";

export function heading5Icon(icon, text) {
    let h5 = document.createElement("h5");
    h5.innerHTML = "<i class='fa fa-" + icon + "'></i> " + text;
    return h5;
}

export function theading(text, colspan=0, tooltip="", id="") {
    let th = document.createElement("th");
    th.classList.add("text-center");
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

export function simpleRow(text, redact=false) {
    let td = document.createElement("td");
    if (redact) {
        td.classList.add("redactable");
    }
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

export function clearDiv(target, targetLength=0) {
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

export function clientTableHeader() {
    let thead = document.createElement("thead");
    thead.appendChild(theading("Circuit"));
    thead.appendChild(theading("Device"));
    thead.appendChild(theading("Plan (Mbps)"));
    thead.appendChild(theading("Parent"));
    thead.appendChild(theading("IP"));
    thead.appendChild(theading("Last Seen"));
    thead.appendChild(theading("Throughput", 2));
    thead.appendChild(theading("RTT", 2));
    thead.appendChild(theading("Re-Xmit", 2));
    return thead;
}

export function formatLastSeen(n) {
    let fiveMinutesInNanos = 300000000000;
    let result = "-";
    if (n > fiveMinutesInNanos) {
        result = "> 5 Minutes ago";
    } else {
        result = scaleNanos(n, 0) + " ago";
    }
    return result;
}

export function topNTableHeader() {
    let th = document.createElement("thead");
    th.classList.add("small");
    th.appendChild(theading("IP Address/Circuit"));
    th.appendChild(theading("Plan"));
    th.appendChild(theading("DL ⬇️"));
    th.appendChild(theading("UL ⬆️"));
    th.appendChild(theading("RTT (ms)"));
    th.appendChild(theading("TCP Retransmits", 2));
    return th;
}

export function topNTableRow(r) {
    let row = document.createElement("tr");
    row.classList.add("small");

    if (r.circuit_id !== "") {
        let link = document.createElement("a");
        link.href = "circuit.html?id=" + encodeURI(r.circuit_id);
        link.innerText = r.ip_address;
        redactCell(link);
        row.append(link);
    } else {
        let ip = document.createElement("td");
        ip.innerText = r.ip_address;
        redactCell(ip);
        row.append(ip);
    }

    let shaped = document.createElement("td");
    shaped.classList.add("tiny");
    shaped.innerText = r.plan.down + " / " + r.plan.up;
    row.append(shaped);

    let dl = document.createElement("td");
    dl.innerHTML = formatThroughput(r.bits_per_second.down, r.plan.down);
    row.append(dl);

    let ul = document.createElement("td");
    ul.innerHTML = formatThroughput(r.bits_per_second.up, r.plan.up);
    row.append(ul);

    let rtt = document.createElement("td");
    rtt.innerHTML = formatRtt(r.median_tcp_rtt);
    row.append(rtt);

    let tcp_xmit_down = document.createElement("td");
    tcp_xmit_down.innerHTML = formatRetransmit(r.tcp_retransmits.down);
    row.append(tcp_xmit_down);

    let tcp_xmit_up = document.createElement("td");
    tcp_xmit_up.innerHTML = formatRetransmit(r.tcp_retransmits.up);
    row.append(tcp_xmit_up);

    return row;
}

export function TopNTableFromMsgData(msg) {
    let t = document.createElement("table");
    t.classList.add("table", "table-striped", "table-sm");

    t.appendChild(topNTableHeader());

    let tbody = document.createElement("tbody");
    msg.data.forEach((r) => {
        t.appendChild(topNTableRow(r));
    });
    t.appendChild(tbody);
    return t;
}