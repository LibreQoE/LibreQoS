import {clearDiv} from "./helpers/builders";
import {scaleNanos, scaleNumber} from "./lq_js_common/helpers/scaling";
import {openFlowRttExcludeWizard} from "./lq_js_common/helpers/flow_rtt_exclude_wizard";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

let asnList = [];
let countryList = [];
let protocolList = [];
let asnListLoaded = false;
let countryListLoaded = false;
let protocolListLoaded = false;
let emptyBannerRendered = false;
let asnData = [];
let graphMinTime = Number.MAX_SAFE_INTEGER;
let graphMaxTime = Number.MIN_SAFE_INTEGER;

const itemsPerPage = 20;
let page = 0;
let renderMode = "asn";

let sortBy = "start";
let sortOptionsList = [
    { tag: "start", label: "Start Time" },
    { tag: "duration", label: "Duration" },
    { tag: "bytes", label: "Bytes" },
];

function unixTimeToDate(unixTime) {
    return new Date(unixTime * 1000).toLocaleString();
}

function maybeRenderEmptyBanner() {
    if (emptyBannerRendered) return;
    if (!asnListLoaded || !countryListLoaded || !protocolListLoaded) return;
    if (asnList.length > 0 || countryList.length > 0 || protocolList.length > 0) return;

    let target = document.getElementById("asnDetails");
    if (!target) return;

    emptyBannerRendered = true;
    clearDiv(target);

    let alert = document.createElement("div");
    alert.classList.add("alert", "alert-info", "mt-2");

    let title = document.createElement("div");
    title.classList.add("fw-semibold");
    title.innerText = "No recent flow data to display.";
    alert.appendChild(title);

    let body = document.createElement("div");
    body.classList.add("small");
    body.innerText =
        "ASN Explorer is populated from recently completed two-way flows (roughly the last 60 seconds). " +
        "On very low traffic networks there may be nothing to show yet. Generate some traffic and refresh.";
    alert.appendChild(body);

    target.appendChild(alert);
}

function renderDropdown({ targetId, buttonText, data, emptyText, buildItem }) {
    let parentDiv = document.createElement("div");
    parentDiv.classList.add("dropdown");

    let button = document.createElement("button");
    button.classList.add("btn", "btn-secondary", "dropdown-toggle", "btn-sm");
    button.type = "button";
    button.innerHTML = buttonText;
    button.setAttribute("data-bs-toggle", "dropdown");
    button.setAttribute("aria-expanded", "false");
    parentDiv.appendChild(button);

    let dropdownList = document.createElement("ul");
    dropdownList.classList.add("dropdown-menu");

    if (!data || data.length === 0) {
        let li = document.createElement("li");
        li.classList.add("dropdown-item", "disabled");
        li.setAttribute("aria-disabled", "true");
        li.tabIndex = -1;
        li.innerText = emptyText || "No recent flow data";
        dropdownList.appendChild(li);
    } else {
        data.forEach((row) => {
            let li = buildItem(row);
            if (li) dropdownList.appendChild(li);
        });
    }

    parentDiv.appendChild(dropdownList);

    let target = document.getElementById(targetId);
    clearDiv(target);
    target.appendChild(parentDiv);
}

function asnDropdown() {
    listenOnce("AsnList", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        asnList = data;
        asnListLoaded = true;

        // Sort data by row.count, descending
        data.sort((a, b) => {
            return b.count - a.count;
        });

        renderDropdown({
            targetId: "asnList",
            buttonText: "Select ASN",
            data,
            emptyText: "No recent flow data",
            buildItem: (row) => {
                if (!row) return null;
                let li = document.createElement("li");
                li.innerText = `#${row.asn} ${row.name} (${row.count})`;
                li.classList.add("dropdown-item");
                li.onclick = () => {
                    renderMode = "asn";
                    selectAsn(row.asn);
                };
                return li;
            },
        });

        maybeRenderEmptyBanner();
    });
    wsClient.send({ AsnList: {} });
}

function countryDropdown() {
    listenOnce("CountryList", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        countryList = data;
        countryListLoaded = true;

        // Sort data by row.count, descending
        data.sort((a, b) => {
            return b.count - a.count;
        });
        //console.log(data);

        renderDropdown({
            targetId: "countryList",
            buttonText: "Select Country",
            data,
            emptyText: "No recent flow data",
            buildItem: (row) => {
                if (!row) return null;
                let li = document.createElement("li");
                li.classList.add("dropdown-item");
                li.onclick = () => {
                    renderMode = "country";
                    selectCountry(row.iso_code);
                };

                let img = document.createElement("img");
                img.alt = row.iso_code;
                img.src = `flags/${row.iso_code.toLowerCase()}.svg`;
                img.height = 12;
                img.width = 12;
                li.appendChild(img);
                li.appendChild(document.createTextNode(` ${row.name} (${row.count})`));
                return li;
            },
        });

        maybeRenderEmptyBanner();
    });
    wsClient.send({ CountryList: {} });
}

function protocolDropdown() {
    listenOnce("ProtocolList", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        protocolList = data;
        protocolListLoaded = true;

        // Sort data by row.count, descending
        data.sort((a, b) => {
            return b.count - a.count;
        });
        //console.log(data);

        renderDropdown({
            targetId: "protocolList",
            buttonText: "Select Protocol",
            data,
            emptyText: "No recent flow data",
            buildItem: (row) => {
                if (!row) return null;
                let li = document.createElement("li");
                li.innerText = `${row.protocol} (${row.count})`;
                li.classList.add("dropdown-item");
                li.onclick = () => {
                    renderMode = "protocol";
                    selectProtocol(row.protocol);
                };
                return li;
            },
        });

        maybeRenderEmptyBanner();
    });
    wsClient.send({ ProtocolList: {} });
}

function selectAsn(asn) {
    listenOnce("AsnFlowTimeline", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        page = 0;
        renderAsn(asn, data);
    });
    wsClient.send({ AsnFlowTimeline: { asn: asn } });
}

function selectCountry(country) {
    listenOnce("CountryFlowTimeline", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        page = 0;
        renderAsn(country, data);
    });
    wsClient.send({ CountryFlowTimeline: { iso_code: country } });
}

function selectProtocol(protocol) {
    listenOnce("ProtocolFlowTimeline", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        page = 0;
        renderAsn(protocol, data);
    });
    wsClient.send({ ProtocolFlowTimeline: { protocol: protocol } });
}

function renderAsn(asn, data) {
    let heading = document.createElement("h2");
    if (renderMode === "asn") {
        let targetAsn = asnList.find((row) => row.asn === asn);
        if (targetAsn === undefined || targetAsn === null) {
            console.error("Could not find ASN: " + asn);
            return;
        }

        // Build the heading
        heading.innerText = "ASN #" + asn.toFixed(0) + " (" + targetAsn.name + ")";
    } else if (renderMode === "country") {
        let targetCountry = countryList.find((row) => row.iso_code === asn);
        if (targetCountry === undefined || targetCountry === null) {
            console.error("Could not find country: " + asn);
            return;
        }

        // Build the heading
        heading.innerHTML = "<img alt='" + targetCountry.iso_code + "' src='flags/" + targetCountry.iso_code.toLowerCase() + ".svg' height=32 width=32 />" + targetCountry.name;
    } else if (renderMode === "protocol") {
        // Build the heading
        heading.innerText = "Protocol: " + asn;
    }

    let target = document.getElementById("asnDetails");

    if (!data || data.length === 0) {
        asnData = [];
        clearDiv(target);
        target.appendChild(heading);

        let alert = document.createElement("div");
        alert.classList.add("alert", "alert-secondary", "mt-2");
        alert.innerText = "No recent flows match this selection yet.";
        target.appendChild(alert);
        return;
    }

    // Get the flow data
    asnData = data;

    // Sort by the selected sort key
    switch (sortBy) {
        case "start": {
            data.sort((a, b) => {
                return a.start - b.start;
            });
        } break;
        case "duration": {
            data.sort((a, b) => {
                return b.duration_nanos - a.duration_nanos;
            });
        } break;
        case "bytes": {
            data.sort((a, b) => {
                return (b.total_bytes.down + b.total_bytes.up) - (a.total_bytes.down + a.total_bytes.up);
            });
        }
    }

    let div = document.createElement("div");
    div.classList.add("row");

    let minTime = Number.MAX_SAFE_INTEGER;
    let maxTime = Number.MIN_SAFE_INTEGER;

    // Calculate time overall
    data.forEach((row) => {
        // Update min/max time
        if (row.start < minTime) {
            minTime = row.start;
        }
        if (row.end > maxTime) {
            maxTime = row.end;
        }
    });

    // Store the global time range
    graphMinTime = minTime;
    graphMaxTime = maxTime;

    // Header row (explain the columns)
    let headerDiv = document.createElement("div");
    headerDiv.classList.add("row");
    let headerBytes = document.createElement("div");
    headerBytes.classList.add("col-1", "text-secondary");
    headerBytes.innerText = "Bytes";
    headerDiv.appendChild(headerBytes);
    let headerRtt = document.createElement("div");
    headerRtt.classList.add("col-1", "text-secondary");
    headerRtt.innerText = "RTT";
    headerDiv.appendChild(headerRtt);
    let headerClient = document.createElement("div");
    headerClient.classList.add("col-1", "text-secondary");
    headerClient.innerText = "Client";
    headerDiv.appendChild(headerClient);
    let headerRemote = document.createElement("div");
    headerRemote.classList.add("col-1", "text-secondary");
    headerRemote.innerText = "Remote";
    headerDiv.appendChild(headerRemote);
    let headerProtocol = document.createElement("div");
    headerProtocol.classList.add("col-1", "text-secondary");
    headerProtocol.innerText = "Protocol";
    headerDiv.appendChild(headerProtocol);
    let headerTime1 = document.createElement("div");
    headerTime1.classList.add("col-4", "text-secondary");
    headerTime1.innerText = unixTimeToDate(minTime);
    headerDiv.appendChild(headerTime1);
    let headerTime2 = document.createElement("div");
    headerTime2.classList.add("col-3", "text-secondary", "text-end");
    headerTime2.innerText = unixTimeToDate(maxTime);
    headerDiv.appendChild(headerTime2);

    let flowsDiv = document.createElement("div");
    for (let i= page * itemsPerPage; i<(page+1) * itemsPerPage; i++) {
        if (i >= data.length) break;
        let row = data[i];

        // Build the headings
        let totalCol = document.createElement("div");
        totalCol.classList.add("col-1", "text-secondary", "small");
        totalCol.innerText = scaleNumber(row.total_bytes.down, 0) + " / " + scaleNumber(row.total_bytes.up);
        div.appendChild(totalCol);

        let rttCol = document.createElement("div");
        rttCol.classList.add("col-1", "text-secondary", "small");
        let rttDown = row.rtt[0] !== undefined ? scaleNanos(row.rtt[0].nanoseconds, 0) : "-";
        let rttUp = row.rtt[1] !== undefined ? scaleNanos(row.rtt[1].nanoseconds, 0) : "-";
        rttCol.innerText = rttDown + " / " + rttUp;
        div.appendChild(rttCol);

        let clientCol = document.createElement("div");
        clientCol.classList.add("col-1", "text-secondary", "small");
        if (row.circuit_id !== "") {
            let clientLink = document.createElement("a");
            clientLink.href = "/circuit.html?id=" + encodeURI(row.circuit_id);
            clientLink.innerText = row.circuit_name;
            clientLink.classList.add("redactable");
            clientLink.style.textOverflow = "ellipsis";
            clientCol.appendChild(clientLink);
        } else {
            clientCol.classList.add("redactable");
            clientCol.innerText = row.circuit_name;
        }
        div.appendChild(clientCol);

        let remoteCol = document.createElement("div");
        remoteCol.classList.add("col-1", "text-secondary", "small");
        const remoteIp = String(row.remote_ip || "").trim();
        remoteCol.appendChild(document.createTextNode(remoteIp));
        if (remoteIp) {
            const btn = document.createElement("button");
            btn.type = "button";
            btn.className = "btn btn-link btn-sm p-0 ms-1";
            btn.title = "Exclude RTT for this remote endpoint (opens Flow Tracking config)";
            btn.innerHTML = "<i class='fa fa-ban'></i>";
            btn.addEventListener("click", (e) => {
                e.preventDefault();
                e.stopPropagation();
                openFlowRttExcludeWizard({ remoteIp, sourceLabel: "ASN Explorer" });
            });
            remoteCol.appendChild(btn);
        }
        div.appendChild(remoteCol);

        let protocolCol = document.createElement("div");
        protocolCol.classList.add("col-1", "text-secondary", "small");
        protocolCol.innerText = row.protocol;
        div.appendChild(protocolCol);

        // Build a canvas div, we'll decorate this later
        let canvasCol = document.createElement("div");
        canvasCol.classList.add("col-7");
        let canvas = document.createElement("canvas");
        canvas.id = "flowCanvas" + i;
        canvas.style.width = "100%";
        canvas.style.height = "20px";
        canvasCol.appendChild(canvas);
        div.appendChild(canvasCol);

        flowsDiv.appendChild(div);
    }

    // Apply the data to the page
    clearDiv(target);
    target.appendChild(heading);

    let nextButton = document.createElement("button");
    nextButton.classList.add("btn", "btn-secondary", "btn-sm", "ms-2");
    nextButton.innerHTML = "<i class='fa fa-arrow-right'></i> Next";
    nextButton.onclick = () => {
        page++;
        if (page * itemsPerPage >= data.length) page = Math.floor(data.length / itemsPerPage);
        renderAsn(asn, data);
    };


    let prevButton = document.createElement("button");
    prevButton.classList.add("btn", "btn-secondary", "btn-sm", "me-2");
    prevButton.innerHTML = "<i class='fa fa-arrow-left'></i> Prev";
    prevButton.onclick = () => {
        page--;
        if (page < 0) page = 0;
        renderAsn(asn, data);
    }

    let paginator = document.createElement("span");
    paginator.classList.add("text-secondary", "small", "ms-2", "me-2");
    paginator.innerText = "Page " + (page + 1) + " of " + Math.ceil(data.length / itemsPerPage);
    paginator.id = "paginator";

    let sortOptions = document.createElement("span");
    sortOptions.classList.add("text-secondary", "small", "ms-2", "me-2");
    sortOptions.innerText = "Sort by: ";

    let sortBox = document.createElement("select");
    sortBox.classList.add("small");
    sortBox.id = "sortBox";
    sortOptionsList.forEach((option) => {
        let opt = document.createElement("option");
        opt.value = option.tag;
        opt.innerText = option.label;
        if (option.tag === sortBy) {
            opt.selected = true;
        }
        sortBox.appendChild(opt);
    });
    sortBox.onchange = () => {
        let sortBox = document.getElementById("sortBox");
        sortBy = sortBox.value;
        renderAsn(asn, data);
    }

    let controlDiv = document.createElement("div");
    controlDiv.classList.add("mb-2");
    controlDiv.appendChild(prevButton);
    controlDiv.appendChild(paginator);
    controlDiv.appendChild(nextButton);
    controlDiv.appendChild(sortOptions);
    controlDiv.appendChild(sortBox);
    target.appendChild(controlDiv);
    target.appendChild(headerDiv);

    target.appendChild(flowsDiv);

    // Wait for the page to render before drawing the graphs
    requestAnimationFrame(() => {
        setTimeout(() => {
            drawTimeline();
        });
    });
}

function timeToX(time, width) {
    let range = graphMaxTime - graphMinTime;
    if (range <= 0) return 0;
    let offset = time - graphMinTime;
    return (offset / range) * width;
}

function drawTimeline() {
    var style = getComputedStyle(document.body)
    let regionBg = style.getPropertyValue('--bs-tertiary-bg');
    let lineColor = style.getPropertyValue('--bs-primary');
    let axisColor = style.getPropertyValue('--bs-secondary');

    for (let i=page * itemsPerPage; i<(page+1)*itemsPerPage; i++) {
        let row = asnData[i];
        //console.log(row);
        let canvasId = "flowCanvas" + i;

        // Get the canvas context
        let canvas = document.getElementById(canvasId);
        if (canvas === null) break;
        const { width, height } = canvas.getBoundingClientRect();
        canvas.width = width;
        canvas.height = height;
        let ctx = canvas.getContext("2d");

        // Draw the background for the time period
        ctx.fillStyle = regionBg;
        ctx.fillRect(timeToX(row.start, width), 0, timeToX(row.end, width), height);

        // Draw red lines for TCP retransmits
        ctx.strokeStyle = "red";
        row.retransmit_times_down.forEach((time) => {
            // Start at y/2, end at y
            ctx.beginPath();
            ctx.moveTo(timeToX(time, width), height / 2);
            ctx.lineTo(timeToX(time, width), height);
            ctx.stroke();
        });
        row.retransmit_times_up.forEach((time) => {
            // Start at 0, end at y/2
            ctx.beginPath();
            ctx.moveTo(timeToX(time, width), 0);
            ctx.lineTo(timeToX(time, width), height / 2);
            ctx.stroke();
        });

        // Draw a horizontal axis line the length of the canvas area at y/2
        ctx.strokeStyle = axisColor;
        ctx.beginPath();
        ctx.moveTo(timeToX(row.start, width), height / 2);
        ctx.lineTo(timeToX(row.end, width), height / 2);
        ctx.stroke();

        // Calculate maxThroughputUp and maxThroughputDown for this row
        let maxThroughputDown = 0;
        let maxThroughputUp = 0;
        row.throughput.forEach((value) => {
            if (value.down > maxThroughputDown) {
                maxThroughputDown = value.down;
            }
            if (value.up > maxThroughputUp) {
                maxThroughputUp = value.up;
            }
        });

        // Draw a throughput down line. Y from y/2 to height, scaled to maxThroughputDown
        ctx.strokeStyle = lineColor;
        ctx.beginPath();
        let numberOfSamples = row.throughput.length;
        let startX = timeToX(row.start, width);
        let endX = timeToX(row.end, width);
        let sampleWidth = (endX - startX) / numberOfSamples;
        let x = timeToX(row.start, width);
        ctx.moveTo(x, height/2);
        let trimmedHeight = height - 4;
        row.throughput.forEach((value) => {
            let downPercent = value.down / maxThroughputDown;
            let y = (height/2) - (downPercent * (trimmedHeight / 2));
            ctx.lineTo(x, y);

            x += sampleWidth;
        });
        ctx.stroke();

        x = timeToX(row.start, width);
        ctx.moveTo(x, height/2);
        row.throughput.forEach((value) => {
            let upPercent = value.up / maxThroughputUp;
            let y = (height/2) + (upPercent * (trimmedHeight / 2));
            ctx.lineTo(x, y);

            x += sampleWidth;
        });
        ctx.stroke();

    }
}

asnDropdown();
countryDropdown();
protocolDropdown();
