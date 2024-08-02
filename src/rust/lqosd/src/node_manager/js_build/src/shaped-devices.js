import {clearDiv, clientTableHeader, formatLastSeen, simpleRow} from "./helpers/builders";
import {subscribeWS} from "./pubsub/ws";
import {formatRetransmit, formatRtt, formatThroughput} from "./helpers/scaling";

let shapedDevices = null;
let displayDevices = null;
let devicesPerPage = 10;
let page = 0;
let searchTerm = "";

function tableRow(device) {
    let tr = document.createElement("tr");
    if (device.circuit_id !== "") {
        tr.id = "row_" + device.circuit_id;
    }
    let link = document.createElement("a");
    link.href = "circuit.html?id=" + device.circuit_id;
    link.innerText = device.circuit_name;
    link.classList.add("redactable");
    let td = document.createElement("td");
    td.appendChild(link);
    tr.appendChild(td);

    link = document.createElement("a");
    link.href = "circuit.html?id=" + device.circuit_id;
    link.innerText = device.device_name;
    link.classList.add("redactable");
    td = document.createElement("td");
    td.appendChild(link);
    tr.appendChild(td);

    tr.appendChild(simpleRow(device.download_max_mbps + " / " + device.upload_max_mbps));
    tr.appendChild(simpleRow(device.parent_node, true));
    let ipList = "";
    device.ipv4.forEach((ip) => {
        ipList += ip[0] + "/" + ip[1] + "<br />";
    });
    device.ipv6.forEach((ip) => {
        ipList += ip[0] + "/" + ip[1] + "<br />";
    });
    let ip = document.createElement("td");
    ip.innerHTML = ipList;
    tr.appendChild(ip);

    let lastSeen = document.createElement("td");
    if (device.circuit_id !== "") {
        lastSeen.id = "lastSeen_" + device.circuit_id;
    }
    lastSeen.innerHTML = "-";
    tr.appendChild(lastSeen);
    let throughputDown = document.createElement("td");
    if (device.circuit_id !== "") {
        throughputDown.id = "throughputDown_" + device.circuit_id;
    }
    throughputDown.innerHTML = "-";
    tr.appendChild(throughputDown);
    let throughputUp = document.createElement("td");
    if (device.circuit_id !== "") {
        throughputUp.id = "throughputUp_" + device.circuit_id;
    }
    throughputUp.innerHTML = "-";
    tr.appendChild(throughputUp);
    let rttDown = document.createElement("td");
    if (device.circuit_id !== "") {
        rttDown.id = "rttDown_" + device.circuit_id;
    }
    rttDown.innerHTML = "-";
    tr.appendChild(rttDown);
    let rttUp = document.createElement("td");
    if (device.circuit_id !== "") {
        rttUp.id = "rttUp_" + device.circuit_id;
    }
    rttUp.innerHTML = "-";
    tr.appendChild(rttUp);
    let reXmitDown = document.createElement("td");
    if (device.circuit_id !== "") {
        reXmitDown.id = "reXmitDown_" + device.circuit_id;
    }
    reXmitDown.innerHTML = "-";
    tr.appendChild(reXmitDown);
    let reXmitUp = document.createElement("td");
    if (device.circuit_id !== "") {
        reXmitUp.id = "reXmitUp_" + device.circuit_id;
    }
    reXmitUp.innerHTML = "-";
    tr.appendChild(reXmitUp);

    return tr;
}

function make_table() {
    let table = document.createElement("table");
    table.classList.add("table", "table-striped");
    table.appendChild(clientTableHeader());
    let tb = document.createElement("tbody");
    let start = page * devicesPerPage;
    let end = Math.min((page + 1) * devicesPerPage, displayDevices.length);
    for (let i = start; i < end; i++) {
        tb.appendChild(tableRow(displayDevices[i]));
    }
    table.appendChild(tb);
    return table;
}

function filterDevices() {
    displayDevices = [];
    let term = searchTerm.toLowerCase();
    shapedDevices.forEach((d) => {
        if (
            d.device_name.toLowerCase().indexOf(term) > -1 ||
            d.circuit_name.toLowerCase().indexOf(term) > -1
        ) {
            displayDevices.push(d);
        }
    });
    page = 0;
    fillTable();
}

function fillTable() {
    let table = make_table();
    let pages = document.createElement("div");
    pages.classList.add("mt-2", "mb-1");

    let numPages = (displayDevices.length / devicesPerPage) - 2;

    if (numPages > 1) {
        if (page > 0) {
            let left = document.createElement("button");
            left.classList.add("btn", "btn-sm", "btn-secondary", "me-2");
            left.innerHTML = "<i class='fa fa-arrow-left'></i>";
            left.onclick = () => {
                page -= 1;
                fillTable();
            }
            pages.appendChild(left);
        }
        let counter = document.createElement("span");
        counter.classList.add("text-secondary");
        counter.innerText = page + " / " + numPages.toFixed(0);
        pages.appendChild(counter);
        if (page < numPages - 2) {
            let right = document.createElement("button");
            right.classList.add("btn", "btn-sm", "btn-secondary", "ms-2");
            right.innerHTML = "<i class='fa fa-arrow-right'></i>";
            right.onclick = () => {
                page += 1;
                fillTable();
            }
            pages.appendChild(right);
        }
    }

    let filter = document.createElement("div");
    //let label = document.createElement("label");
    //label.classList.add("text-secondary");
    //label.innerText = "Search";
    //label.htmlFor = "sdSearch";
    let sdSearch = document.createElement("input");
    sdSearch.id = "sdSearch";
    sdSearch.placeholder = "Search";
    sdSearch.value = searchTerm;
    sdSearch.onkeydown = (event) => {
        if (event.keyCode == 13) {
            searchTerm = $("#sdSearch").val();
            filterDevices();
        }
    }
    let searchButton = document.createElement("button");
    searchButton.type = "button"
    searchButton.classList.add("btn", "btn-sm");
    searchButton.innerHTML = "<i class='fa fa-search'></i>";
    searchButton.onchange = () => {
        searchTerm = $("#sdSearch").val();
        filterDevices();
    }
    //filter.appendChild(label);
    filter.appendChild(sdSearch);
    filter.appendChild(searchButton);

    let target = document.getElementById("deviceTable");
    clearDiv(target);
    target.appendChild(filter);
    target.appendChild(pages);
    target.appendChild(table);
}

function countCircuits() {
    let entries = {};
    shapedDevices.forEach((d) => {
        if (!entries.hasOwnProperty(d.circuit_id)) {
            entries[d.circuit_id] = 1;
        }
    })
    let count = 0;
    for (const i in entries) {
        count++;
    }
    return count;
}

function loadDevices() {
    $.get("/local-api/devicesAll", (data) => {
        //console.log(data);
        shapedDevices = data;
        displayDevices = data;
        fillTable();
        $("#count").text(shapedDevices.length + " devices");
        $("#countCircuit").text(countCircuits() + " circuits");
    })
}

loadDevices();
subscribeWS(["NetworkTreeClients"], (msg) => {
    if (msg.event === "NetworkTreeClients") {
        //console.log(msg);
        msg.data.forEach((d) => {
            let lastSeen = document.getElementById("lastSeen_" + d.circuit_id);
            if (lastSeen !== null) {
                lastSeen.innerText = formatLastSeen(d.last_seen_nanos);
            }
            let throughputDown = document.getElementById("throughputDown_" + d.circuit_id);
            if (throughputDown !== null) {
                throughputDown.innerHTML = formatThroughput(d.bytes_per_second.down * 8, d.plan.down);
            }
            let throughputUp = document.getElementById("throughputUp_" + d.circuit_id);
            if (throughputUp !== null) {
                throughputUp.innerHTML = formatThroughput(d.bytes_per_second.up * 8, d.plan.up);
            }
            let rttDown = document.getElementById("rttDown_" + d.circuit_id);
            if (rttDown !== null && d.median_latency != null) {
                rttDown.innerHTML = formatRtt(d.median_latency.down);
            }
            let rttUp = document.getElementById("rttUp_" + d.circuit_id);
            if (rttUp !== null && d.median_latency != null) {
                rttUp.innerHTML = formatRtt(d.median_latency.up);
            }
            let reXmitDown = document.getElementById("reXmitDown_" + d.circuit_id);
            if (reXmitDown !== null) {
                reXmitDown.innerHTML = formatRetransmit(d.tcp_retransmits.down);
            }
            let reXmitUp = document.getElementById("reXmitUp_" + d.circuit_id);
            if (reXmitUp !== null) {
                reXmitUp.innerHTML = formatRetransmit(d.tcp_retransmits.up);
            }
        });
    }
});