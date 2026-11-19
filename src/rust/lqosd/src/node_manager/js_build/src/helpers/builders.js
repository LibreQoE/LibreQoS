import {
    formatRetransmitFraction,
    formatRtt,
    formatThroughput,
    retransmitFractionFromSample,
} from "./scaling";
import {redactCell} from "./redact";
import {disposeTooltipsWithin, enableTooltipsWithin} from "../lq_js_common/helpers/tooltips";
import {clearElement} from "./dom_clear.mjs";
import {t} from "../i18n/index.js";
export {safeRelativeHref, simpleLinkRow} from "./safe_dom.mjs";
export {formatLastSeen} from "./last_seen.mjs";

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
        th.innerHTML = text;
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

export function simpleRowTrustedHtml(text) {
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
    clearElement(target, targetLength);
}

export function enableTooltips() {
    enableTooltipsWithin(document);
}

let pendingTooltips = [];

export function tooltipsNextFrame(id) {
    pendingTooltips.push(id);
    requestAnimationFrame(() => {
        setTimeout(() => {
            pendingTooltips.forEach((id) => {
                let tooltipTriggerEl = document.getElementById(id);
                if (tooltipTriggerEl !== null) {
                    enableTooltipsWithin(tooltipTriggerEl);
                }
            });
            pendingTooltips = [];
        })
    });
}

export function clientTableHeader() {
    let thead = document.createElement("thead");
    thead.appendChild(theading(t("common.table.circuit")));
    thead.appendChild(theading(t("common.table.device")));
    const planHeading = theading(t("common.table.plan_mbps"));
    planHeading.classList.add("lqos-tree-plan-column");
    thead.appendChild(planHeading);
    thead.appendChild(theading(t("common.table.parent")));
    thead.appendChild(theading("IP"));
    thead.appendChild(theading(t("common.table.last_seen")));
    thead.appendChild(theading(t("common.table.throughput"), 2));
    thead.appendChild(theading("RTT", 2));
    thead.appendChild(theading(t("common.table.rxmit_short"), 2));
    return thead;
}

// `formatLastSeen` lives in `./last_seen.mjs` and is re-exported above: it
// composes the relative-time wording through `i18n/relative_time.mjs`, which a
// Node test can only reach from a module that has no extensionless imports.

export function topNTableHeader() {
    let th = document.createElement("thead");
    th.classList.add("small");
    th.appendChild(theading(t("common.table.ip_address_circuit")));
    th.appendChild(theading(t("common.table.plan")));
    th.appendChild(theading("DL <i class='fa fa-arrow-down'></i>"));
    th.appendChild(theading("UL <i class='fa fa-arrow-up'></i>"));
    th.appendChild(theading(t("common.table.rtt_ms")));
    th.appendChild(theading(t("common.table.tcp_retransmits"), 2));
    return th;
}

function formatPlanValue(value) {
    // Format plan value to max 3 decimal places, removing trailing zeros
    let formatted = parseFloat(value).toFixed(3);
    // Remove trailing zeros after decimal point
    formatted = formatted.replace(/\.?0+$/, '');
    return formatted;
}

export function topNTableRow(r) {
    let row = document.createElement("tr");
    row.classList.add("small");
    let opacity = (10.0 - r.lastSeen) / 10.0;
    //console.log(opacity, r.lastSeen);
    row.style.opacity = opacity.toFixed(1);

    if (r.circuit_id !== "") {
        let td = document.createElement("td");
        let link = document.createElement("a");
        link.href = "circuit.html?id=" + encodeURI(r.circuit_id);
        link.innerText = r.ip_address;
        redactCell(link);
        td.append(link);
        row.append(td);
    } else {
        let ip = document.createElement("td");
        ip.innerText = r.ip_address;
        redactCell(ip);
        row.append(ip);
    }

    let shaped = document.createElement("td");
    shaped.innerText = formatPlanValue(r.plan.down) + " / " + formatPlanValue(r.plan.up);
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
    tcp_xmit_down.innerHTML = formatRetransmitFraction(
        retransmitFractionFromSample(r.tcp_retransmit_sample?.down)
    );
    row.append(tcp_xmit_down);

    let tcp_xmit_up = document.createElement("td");
    tcp_xmit_up.innerHTML = formatRetransmitFraction(
        retransmitFractionFromSample(r.tcp_retransmit_sample?.up)
    );
    row.append(tcp_xmit_up);

    return row;
}

export function TopNTableFromMsgData(circuits) {
    let t = document.createElement("table");
    t.classList.add("dash-table", "lqos-table", "lqos-table-compact", "lqos-topn-plain");

    t.appendChild(topNTableHeader());

    let tbody = document.createElement("tbody");
    circuits.forEach((r) => {
        tbody.appendChild(topNTableRow(r));
    });
    t.appendChild(tbody);
    return t;
}
