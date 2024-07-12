// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";
import {clearDiv, formatLastSeen} from "./helpers/builders";
import {formatRetransmit, formatRtt, formatThroughput} from "./helpers/scaling";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {CircuitTotalGraph} from "./graphs/circuit_throughput_graph";
import {CircuitRetransmitGraph} from "./graphs/circuit_retransmit_graph";

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let circuit_id = decodeURI(params.id);
let channelLink = null;
let speedometer = null;
let totalThroughput = null;
let totalRetransmits = null;
let deviceGraphs = {};

function connectPrivateChannel() {
    channelLink = new DirectChannel({
        CircuitWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        if (msg.devices !== null) {
            //console.log(msg.devices);
            fillLiveDevices(msg.devices);
            updateSpeedometer(msg.devices);
        }
    });
}

function updateSpeedometer(devices) {
    let totalDown = 0;
    let totalUp = 0;
    let planDown = 0;
    let planUp = 0;
    let retransmitsDown = 0;
    let retransmitsUp = 0;
    devices.forEach((device) => {
        totalDown += device.bytes_per_second.down;
        totalUp += device.bytes_per_second.up;
        planDown = Math.max(planDown, device.plan.down);
        planUp = Math.max(planUp, device.plan.up);
        retransmitsDown += device.tcp_retransmits.down;
        retransmitsUp += device.tcp_retransmits.up;

        let throughputGraph = deviceGraphs["throughputGraph_" + device.device_id];
        if (throughputGraph !== undefined) {
            throughputGraph.update(device.bytes_per_second.down * 8, device.bytes_per_second.up * 8);
        }

        let retransmitGraph = deviceGraphs["tcpRetransmitsGraph_" + device.device_id];
        if (retransmitGraph !== undefined) {
            retransmitGraph.update(device.tcp_retransmits.down, device.tcp_retransmits.up);
        }
    });
    speedometer.update(totalDown * 8, totalUp * 8, planDown, planUp);
    totalThroughput.update(totalDown * 8, totalUp * 8);
    totalRetransmits.update(retransmitsDown, retransmitsUp);
}

function fillLiveDevices(devices) {
    devices.forEach((device) => {
        let last_seen = document.getElementById("last_seen_" + device.device_id);
        let throughputDown = document.getElementById("throughputDown_" + device.device_id);
        let throughputUp = document.getElementById("throughputUp_" + device.device_id);
        let rttDown = document.getElementById("rttDown_" + device.device_id);
        let rttUp = document.getElementById("rttUp_" + device.device_id);
        let tcp_retransmitsDown = document.getElementById("tcp_retransmitsDown_" + device.device_id);
        let tcp_retransmitsUp = document.getElementById("tcp_retransmitsUp_" + device.device_id);

        if (last_seen !== null) {
            last_seen.innerHTML = formatLastSeen(device.last_seen_nanos);
        }

        if (throughputDown !== null) {
            throughputDown.innerHTML = formatThroughput(device.bytes_per_second.down * 8, device.plan.down);
        }

        if (throughputUp !== null) {
            throughputUp.innerHTML = formatThroughput(device.bytes_per_second.up * 8, device.plan.up);
        }

        if (rttDown !== null) {
            if (device.median_latency !== null && device.median_latency.down !== null) {
                rttDown.innerHTML = formatRtt(device.median_latency.down);
            }
        }

        if (rttUp !== null) {
            if (device.median_latency !== null && device.median_latency.up !== null) {
                rttUp.innerHTML = formatRtt(device.median_latency.up);
            }
        }

        if (tcp_retransmitsDown !== null) {
            tcp_retransmitsDown.innerHTML = formatRetransmit(device.tcp_retransmits.down);
        }

        if (tcp_retransmitsUp !== null) {
            tcp_retransmitsUp.innerHTML = formatRetransmit(device.tcp_retransmits.up);
        }
    });
}

function initialDevices(circuits) {
    let target = document.getElementById("devices")
    clearDiv(target);

    circuits.forEach((circuit) => {
        let d = document.createElement("div");
        d.classList.add("col-3");

        // Device Information Section

        let name = document.createElement("h5");
        name.innerHTML = "<i class='fa fa-computer'></i> " + circuit.device_name;
        d.appendChild(name);

        let infoTable = document.createElement("table");
        infoTable.classList.add("table", "table-sm", "table-striped");
        let tbody = document.createElement("tbody");

        // MAC Row
        let tr = document.createElement("tr");
        let td = document.createElement("td");
        td.innerHTML = "<b>MAC Address</b>";
        tr.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.innerHTML = circuit.mac;
        tr.appendChild(td);
        tbody.appendChild(tr);

        // Comment Row
        let tr2 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Comment</b>";
        tr2.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.innerHTML = circuit.comment;
        tr2.appendChild(td);
        tbody.appendChild(tr2);

        // IPv4 Row
        let tr3 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>IPv4 Address(es)</b>";
        tr3.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        let ipv4 = "";
        circuit.ipv4.forEach((ip) => {
            ipv4 += ip[0] + "/" + ip[1] + "<br>";
        });
        if (ipv4 === "") ipv4 = "No IPv4 addresses assigned";
        td.innerHTML = ipv4;
        tr3.appendChild(td);
        tbody.appendChild(tr3);

        // IPv6 Row
        let tr4 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>IPv6 Address(es)</b>";
        tr4.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        let ipv6 = "";
        circuit.ipv6.forEach((ip) => {
            ipv6 += ip[0] + "/" + ip[1] + "<br>";
        });
        if (ipv6 === "") ipv6 = "No IPv6 addresses assigned";
        td.innerHTML = ipv6;
        tr4.appendChild(td);
        tbody.appendChild(tr4);

        // Placeholder for Last Seen
        let tr8 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Last Seen</b>";
        tr8.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;
        td.id = "last_seen_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr8.appendChild(td);
        tbody.appendChild(tr8);

        // Placeholder for throughput
        let tr5 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>Throughput</b>";
        tr5.appendChild(td);
        td = document.createElement("td");
        td.id = "throughputDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        td = document.createElement("td");
        td.id = "throughputUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr5.appendChild(td);
        tbody.appendChild(tr5);

        // Placeholder for RTT
        let tr6 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>RTT</b>";
        tr6.appendChild(td);
        td = document.createElement("td");
        td.id = "rttDown_" + circuit.device_id;
        td.innerHTML = "Sampling...";
        tr6.appendChild(td);
        td = document.createElement("td");
        td.id = "rttUp_" + circuit.device_id;
        td.innerHTML = "Sampling...";
        tr6.appendChild(td);
        tbody.appendChild(tr6);

        // Placeholder for TCP Retransmits
        let tr7 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>TCP Retransmits</b>";
        tr7.appendChild(td);
        td = document.createElement("td");
        td.id = "tcp_retransmitsDown_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        td = document.createElement("td");
        td.id = "tcp_retransmitsUp_" + circuit.device_id;
        td.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        tr7.appendChild(td);
        tbody.appendChild(tr7);

        infoTable.appendChild(tbody);
        d.appendChild(infoTable);
        target.appendChild(d);

        // Graph for Throughput
        let throughputGraph = document.createElement("div");
        throughputGraph.classList.add("col-4")
        throughputGraph.id = "throughputGraph_" + circuit.device_id;
        throughputGraph.style.height = "250px";
        throughputGraph.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        target.appendChild(throughputGraph);
        deviceGraphs[throughputGraph.id] = new CircuitTotalGraph(throughputGraph.id, circuit.device_name + " Throughput");

        // Graph for TCP Retransmits
        let tcpRetransmitsGraph = document.createElement("div");
        tcpRetransmitsGraph.classList.add("col-4")
        tcpRetransmitsGraph.id = "tcpRetransmitsGraph_" + circuit.device_id;
        tcpRetransmitsGraph.style.height = "250px";
        tcpRetransmitsGraph.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
        target.appendChild(tcpRetransmitsGraph);
        deviceGraphs[tcpRetransmitsGraph.id] = new CircuitRetransmitGraph(tcpRetransmitsGraph.id, circuit.device_name + " Retransmits");
    });
}

function loadInitial() {
    $.ajax({
        type: "POST",
        url: "/local-api/circuitById",
        data: JSON.stringify({ id: circuit_id }),
        contentType: 'application/json',
        success: (circuits) => {
            console.log(circuits);
            let circuit = circuits[0];
            $("#circuitName").text(circuit.circuit_name);
            $("#parentNode").text(circuit.parent_node);
            $("#bwMax").text(circuit.download_max_mbps + " / " + circuit.upload_max_mbps + " Mbps");
            $("#bwMin").text(circuit.download_min_mbps + " / " + circuit.upload_min_mbps + " Mbps");
            initialDevices(circuits);
            speedometer = new BitsPerSecondGauge("bitsGauge");
            totalThroughput = new CircuitTotalGraph("throughputGraph", "Total Circuit Throughput");
            totalRetransmits = new CircuitRetransmitGraph("rxmitGraph", "Total Circuit Retransmits");

            connectPrivateChannel();
        },
        error: () => {
            alert("Circuit with id " + circuit_id + " not found");
        }
    })
}

loadInitial();