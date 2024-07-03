import {clearDiv, heading5Icon, simpleRow, theading} from "./helpers/builders";

let shapedDevices = null;
let displayDevices = null;
let devicesPerPage = 10;
let page = 0;
let searchTerm = "";

function tableRow(device) {
    let tr = document.createElement("tr");
    tr.appendChild(simpleRow(device.circuit_name));
    tr.appendChild(simpleRow(device.device_name));
    tr.appendChild(simpleRow(device.download_max_mbps + " / " + device.upload_max_mbps));
    tr.appendChild(simpleRow(device.parent_node));
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
    return tr;
}

function make_table() {
    let table = document.createElement("table");
    table.classList.add("table", "table-striped");
    let thead = document.createElement("thead");
    thead.appendChild(theading("Circuit"));
    thead.appendChild(theading("Device"));
    thead.appendChild(theading("Plan (Mbps)"));
    thead.appendChild(theading("Parent"));
    thead.appendChild(theading("IP"));
    table.appendChild(thead);
    let tb = document.createElement("tbody");
    let start = page * devicesPerPage;
    let end = Math.min((page + 1) * devicesPerPage, displayDevices.length);
    for (let i=start; i<end; i++) {
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

    let numPages = (displayDevices.length / devicesPerPage)-2;

    if (numPages > 1) {
        if (page > 0) {
            let left = document.createElement("button");
            left.classList.add("btn", "btn-sm", "btn-primary");
            left.innerHTML = "<i class='fa fa-arrow-left'></i>";
            left.onclick = () => {
                page -= 1;
                fillTable();
            }
            pages.appendChild(left);
        }
        let counter = document.createElement("span");
        counter.innerText = page + " / " + numPages.toFixed(0);
        pages.appendChild(counter);
        if (page < numPages - 2) {
            let right = document.createElement("button");
            right.classList.add("btn", "btn-sm", "btn-primary");
            right.innerHTML = "<i class='fa fa-arrow-right'></i>";
            right.onclick = () => {
                page += 1;
                fillTable();
            }
            pages.appendChild(right);
        }
    }

    let filter = document.createElement("div");
    let label = document.createElement("label");
    label.innerText = "Search";
    label.htmlFor = "sdSearch";
    let sdSearch = document.createElement("input");
    sdSearch.id = "sdSearch";
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
    filter.appendChild(label);
    filter.appendChild(sdSearch);
    filter.appendChild(searchButton);

    let target = document.getElementById("deviceTable");
    clearDiv(target);
    target.appendChild(filter);
    target.appendChild(table);
    target.appendChild(pages);
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