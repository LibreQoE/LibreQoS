import {clearDiv} from "./helpers/builders";
import {scaleNanos, scaleNumber} from "./helpers/scaling";

//const API_URL = "local-api/";
const API_URL = "local-api/";
const LIST_URL = API_URL + "asnList";
const FLOW_URL = API_URL + "flowTimeline/";

let asnList = [];
let asnData = [];
let graphMinTime = Number.MAX_SAFE_INTEGER;
let graphMaxTime = Number.MIN_SAFE_INTEGER;

function asnDropdown() {
    $.get(LIST_URL, (data) => {
        asnList = data;

        // Sort data by row.count, descending
        data.sort((a, b) => {
            return b.count - a.count;
        });

        // Build the dropdown
        let parentDiv = document.createElement("div");
        parentDiv.classList.add("dropdown");
        let button = document.createElement("button");
        button.classList.add("btn", "btn-secondary", "dropdown-toggle");
        button.type = "button";
        button.innerHTML = "Select ASN";
        button.setAttribute("data-bs-toggle", "dropdown");
        button.setAttribute("aria-expanded", "false");
        parentDiv.appendChild(button);
        let dropdownList = document.createElement("ul");
        dropdownList.classList.add("dropdown-menu");

        // Add items
        data.forEach((row) => {
            let li = document.createElement("li");
            li.innerHTML = row.name + " (" + row.count + ")";
            li.classList.add("dropdown-item");
            li.onclick = () => {
                selectAsn(row.asn);
            };
            dropdownList.appendChild(li);
        });

        parentDiv.appendChild(dropdownList);
        let target = document.getElementById("asnList");
        clearDiv(target);
        target.appendChild(parentDiv);

        /*if (data.length > 0) {
            selectAsn(data[0].asn);
        }*/
    });
}

function selectAsn(asn) {
    let targetAsn = asnList.find((row) => row.asn === asn);
    if (targetAsn === undefined || targetAsn === null) {
        console.error("Could not find ASN: " + asn);
        return;
    }

    let target = document.getElementById("asnDetails");

    // Build the heading
    let heading = document.createElement("h2");
    heading.innerText = "ASN #" + asn.toFixed(0) + " (" + targetAsn.name + ")";

    // Get the flow data
    $.get(FLOW_URL + asn, (data) => {
        // If data has more than 20 entries, only show the first 20 (temporary)
        if (data.length > 20) {
            data = data.slice(0, 20);
        }
        asnData = data;

        // Sort data by row.start, ascending
        data.sort((a, b) => {
            return a.start - b.start;
        });

        // Build the flows display
        let flowsDiv = document.createElement("div");
        let count = 0;
        let minTime = Number.MAX_SAFE_INTEGER;
        let maxTime = Number.MIN_SAFE_INTEGER;
        data.forEach((row) => {
            // Update min/max time
            if (row.start < minTime) {
                minTime = row.start;
            }
            if (row.end > maxTime) {
                maxTime = row.end;
            }

            let div = document.createElement("div");
            div.classList.add("row");

            // Build the heading
            let headingCol = document.createElement("div");
            headingCol.classList.add("col-1");

            let ht = "<p class='text-secondary small'>" + scaleNumber(row.total_bytes.down, 0) + " / " + scaleNumber(row.total_bytes.up);

            if (row.rtt[0] !== undefined) {
                ht += "<br /> RTT: " + scaleNanos(row.rtt[0].nanoseconds, 0);
            } else {
                ht += "<br /> RTT: -";
            }
            if (row.rtt[1] !== undefined) {
                ht += " / " + scaleNanos(row.rtt[1].nanoseconds, 0);
            }
            ht += "</p>";
            headingCol.innerHTML = ht;
            div.appendChild(headingCol);

            // Build a canvas div, we'll decorate this later
            let canvasCol = document.createElement("div");
            canvasCol.classList.add("col-11");
            let canvas = document.createElement("canvas");
            canvas.id = "flowCanvas" + count;
            canvas.style.width = "100%";
            canvas.style.height = "30px";
            canvasCol.appendChild(canvas);
            div.appendChild(canvasCol);

            flowsDiv.appendChild(div);
            count++;
        });

        // Store the global time range
        graphMinTime = minTime;
        graphMaxTime = maxTime;

        // Apply the data to the page
        clearDiv(target);
        target.appendChild(heading);
        target.appendChild(flowsDiv);

        // Wait for the page to render before drawing the graphs
        requestAnimationFrame(() => {
            setTimeout(() => {
                drawTimeline();
            });
        });
    });
}

function timeToX(time, width) {
    let range = graphMaxTime - graphMinTime;
    let offset = time - graphMinTime;
    return (offset / range) * width;
}

function drawTimeline() {
    var style = getComputedStyle(document.body)
    let regionBg = style.getPropertyValue('--bs-tertiary-bg');
    let lineColor = style.getPropertyValue('--bs-primary');

    for (let i=0; i<asnData.length; i++) {
        let row = asnData[i];
        console.log(row);
        let canvasId = "flowCanvas" + i;

        // Get the canvas context
        let canvas = document.getElementById(canvasId);
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

        // Find the max of row.throughput.down and row.throughput.up
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
        let duration = row.end - row.start;
        let numberOfSamples = row.throughput.length;
        let startX = timeToX(row.start, width);
        let endX = timeToX(row.end, width);
        let sampleWidth = (endX - startX) / numberOfSamples;
        let x = timeToX(row.start, width);
        row.throughput.forEach((value, index) => {
            let downPercent = value.down / maxThroughputDown;
            let downHeight = downPercent * (height / 2);
            let y = height - downHeight;
            ctx.moveTo(x, y);

            let upPercent = value.up / maxThroughputUp;
            let upHeight = upPercent * (height / 2);
            ctx.lineTo(x, upHeight);

            x += sampleWidth;
        });
        ctx.stroke();

    }
}

asnDropdown();
