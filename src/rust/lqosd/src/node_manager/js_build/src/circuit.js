// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";
import {clearDiv, formatLastSeen, simpleRow, simpleRowHtml, theading} from "./helpers/builders";
import {formatRetransmit, formatRtt, formatThroughput, lerpGreenToRedViaOrange, formatMbps} from "./helpers/scaling";
import {colorByQoqScore} from "./helpers/color_scales";
import {BitsPerSecondGauge} from "./graphs/bits_gauge";
import {QooScoreGauge} from "./graphs/qoo_score_gauge";
import {CircuitTotalGraph} from "./graphs/circuit_throughput_graph";
import {CircuitRetransmitGraph} from "./graphs/circuit_retransmit_graph";
import {scaleNanos, scaleNumber, toNumber} from "./lq_js_common/helpers/scaling";
import {DevicePingHistogram} from "./graphs/device_ping_graph";
import {WindowedLatencyHistogram} from "./graphs/windowed_latency_histogram";
import {FlowsSankey} from "./graphs/flow_sankey";
import {get_ws_client, subscribeWS} from "./pubsub/ws";
import {CakeBacklog} from "./graphs/cake_backlog";
import {CakeDelays} from "./graphs/cake_delays";
import {CakeQueueLength} from "./graphs/cake_queue_length";
import {CakeTraffic} from "./graphs/cake_traffic";
import {CakeMarks} from "./graphs/cake_marks";
import {CakeDrops} from "./graphs/cake_drops";

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let circuit_id = decodeURI(params.id);
let plan = null;
let channelLink = null;
let pinger = null;
let flowChannel = null;
let speedometer = null;
let qooGauge = null;
let totalThroughput = null;
let totalRetransmits = null;
let deviceGraphs = {};
let devicePings = [];
let flowSankey = null;
let funnelGraphs = {};
let funnelParents = [];
const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function formatIpBytes(bytes) {
    const list = Array.from(bytes);
    if (list.length === 4) {
        return list.join(".");
    }
    if (list.length === 16) {
        const parts = [];
        for (let i = 0; i < list.length; i += 2) {
            const part = (list[i] << 8) | list[i + 1];
            parts.push(part.toString(16).padStart(4, "0"));
        }
        return parts.join(":");
    }
    return list.join(".");
}

function ipToString(ip) {
    if (typeof ip === "string") {
        return ip;
    }
    if (ip instanceof Uint8Array || Array.isArray(ip)) {
        return formatIpBytes(ip);
    }
    return String(ip);
}

function requestCircuitById(onSuccess, onError) {
    listenOnce("CircuitByIdResult", (msg) => {
        if (!msg || !msg.ok) {
            if (onError) onError();
            return;
        }
        if (msg.id && msg.id !== circuit_id) {
            if (onError) onError();
            return;
        }
        onSuccess(msg.devices || []);
    });
    wsClient.send({ CircuitById: { id: circuit_id } });
}

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
            if (qooGauge !== null) {
                qooGauge.update(msg.qoo_score);
            }
        }
    });
}

function fullIpList(circuits) {
    let ipList = [];
    circuits.forEach((circuit) => {
        circuit.ipv4.forEach((ip) => {
            ipList.push([ipToString(ip[0]), circuit.device_id]);
        });
        circuit.ipv6.forEach((ip) => {
            ipList.push([ipToString(ip[0]), circuit.device_id]);
        });
    });
    return ipList;
}

function connectPingers(circuits) {
    let ipList = fullIpList(circuits);

    pinger = new DirectChannel({
        PingMonitor: {
            ips: ipList
        }
    },(msg) => {
        //console.log(msg);
        if (msg.ip != null && msg.ip !== "test") {
            // Stats Updates
            if (devicePings[msg.ip] === undefined) {
                devicePings[msg.ip] = {
                    count: 0,
                    timeout: 0,
                    success: 0,
                    times: [],
                }
            }

                devicePings[msg.ip].count++;
                if (msg.result === "NoResponse") {
                    devicePings[msg.ip].timeout++;
                } else {
                    devicePings[msg.ip].success++;
                    const pingNanos = toNumber(msg.result.Ping.time_nanos, 0);
                    devicePings[msg.ip].times.push(pingNanos);
                    if (devicePings[msg.ip].times.length > 300) {
                        devicePings[msg.ip].times.shift();
                    }
                    let graphId = "pingGraph_" + msg.result.Ping.label;
                    let graph = deviceGraphs[graphId];
                    if (graph !== undefined) {
                        graph.update(pingNanos);
                    }
                }

            // Visual Updates
            let target = document.getElementById("ip_" + msg.ip);
            if (target != null) {
                let myPing = devicePings[msg.ip];
                if (myPing.count === myPing.timeout) {
                    target.innerHTML = "<i class='fa fa-minus-circle text-secondary' data-bs-toggle='tooltip' data-bs-placement='top' title='No ping response - this is normal for many ISPs'></i>";
                } else {
                    let loss = ((myPing.timeout / myPing.count) * 100);
                    let lossStr = loss.toFixed(1);
                    let avg = 0;
                    myPing.times.forEach((time) => {
                        avg += time;
                    });
                    avg = avg / myPing.times.length;
                    let lossColor = "text-success";
                    if (loss > 0 && loss < 10) {
                        lossColor = "text-warning";
                    } else if (loss >= 10) {
                        lossColor = "text-danger";
                    }
                    let pingRamp = Math.min(avg / 200, 1);
                    let pingColor = lerpGreenToRedViaOrange(pingRamp, 1);
                    target.innerHTML = "<i class='fa fa-check text-success' data-bs-toggle='tooltip' data-bs-placement='top' title='Device is responding to pings'></i> <span class='tiny'><span class='" + lossColor + "'>" + lossStr + "%</span> / <span style='color: " + pingColor + "'>" + scaleNanos(avg) + "</span></span>";
                }
                // Initialize Bootstrap tooltips
                const tooltipTriggerList = target.querySelectorAll('[data-bs-toggle="tooltip"]');
                const tooltipList = [...tooltipTriggerList].map(tooltipTriggerEl => new bootstrap.Tooltip(tooltipTriggerEl));
            }
        }
    });
}

function connectFlowChannel() {
    flowChannel = new DirectChannel({
        FlowsByCircuit: {
            circuit: circuit_id
        }
    }, (msg) => {
        //console.log(msg);
        let activeFlows = flowSankey.update(msg);
        flowSankey.chart.resize();
        $("#activeFlowCount").text(activeFlows);
        updateTrafficTab(msg);
    });
}

let movingAverages = new Map();
let prevFlowBytes = new Map();
let tickCount = 0;
let trafficSortColumn = 'rate'; // Default sort by rate
let trafficSortDirection = 'desc'; // 'asc' or 'desc'

function diffToNumber(current, previous, fallback = 0) {
    if (typeof current === "bigint" && typeof previous === "bigint") {
        return toNumber(current - previous, fallback);
    }
    return toNumber(current, fallback) - toNumber(previous, fallback);
}

function formatQooScore(score0to100, fallback = "-") {
    if (score0to100 === null || score0to100 === undefined) {
        return fallback;
    }
    const numeric = Number(score0to100);
    // QoqScores uses 255 for unknown.
    if (!Number.isFinite(numeric) || numeric === 255) {
        return fallback;
    }
    const clamped = Math.min(100, Math.max(0, Math.round(numeric)));
    const color = colorByQoqScore(clamped);
    return "<span class='muted' style='color: " + color + "'>■</span>" + clamped;
}

function formatRttNanos(rttNanos) {
    const n = toNumber(rttNanos, 0);
    if (n === 0) {
        return "<span class='muted' style='color: var(--bs-border-color)'>■</span>-";
    }
    const rttInMs = Math.min(200, n / 1000000);
    const color = lerpGreenToRedViaOrange(200 - rttInMs, 200);
    return "<span class='muted' style='color: " + color + "'>■</span>" + scaleNanos(n);
}

function formatRttPair(p50Nanos, p95Nanos) {
    const p50 = toNumber(p50Nanos, 0);
    const p95 = toNumber(p95Nanos, 0);
    if (p50 === 0 && p95 === 0) {
        return "-";
    }
    return formatRttNanos(p50) + " / " + scaleNanos(p95);
}

function updateTrafficTab(msg) {
    let target = document.getElementById("allTraffic");

    let table = document.createElement("table");
    table.classList.add("table", "table-sm", "table-striped");
    let thead = document.createElement("thead", "small");
    thead.style.fontSize = "0.8em";
    
    // Create clickable headers
    const createSortableHeader = (text, sortKey, colspan = 1) => {
        let th = theading(text, colspan);
        th.style.cursor = "pointer";
        th.onclick = () => {
            if (trafficSortColumn === sortKey) {
                trafficSortDirection = trafficSortDirection === 'asc' ? 'desc' : 'asc';
            } else {
                trafficSortColumn = sortKey;
                trafficSortDirection = 'desc';
            }
        };
        // Add sort indicator
        if (trafficSortColumn === sortKey) {
            th.innerHTML += trafficSortDirection === 'asc' ? ' ▲' : ' ▼';
        }
        return th;
    };
    
    thead.appendChild(createSortableHeader("Protocol", "protocol"));
    thead.appendChild(createSortableHeader("Current Rate (d/u)", "rate", 2));
    thead.appendChild(createSortableHeader("Total Bytes (d/u)", "bytes", 2));
    thead.appendChild(createSortableHeader("Total Packets (d/u)", "packets", 2));
    thead.appendChild(createSortableHeader("TCP rxmit (d/u)", "retransmits", 2));
    thead.appendChild(createSortableHeader("RTT (d/u)", "rtt", 2));
    thead.appendChild(createSortableHeader("QoO (d/u)", "qoo", 2));
    thead.appendChild(createSortableHeader("ASN", "asn"));
    thead.appendChild(createSortableHeader("Country", "country"));
    thead.appendChild(createSortableHeader("Remote IP", "ip"));
    table.appendChild(thead);
    let tbody = document.createElement("tbody");
    const thirty_seconds_in_nanos = 30000000000; // For display filtering
    tickCount++;
    
    let hideSmallFlows = document.getElementById("hideSmallFlows").checked;
    let tableRows = [];

    msg.flows.forEach((flow) => {
        let flowKey = flow[0].protocol_name + flow[0].row_id;
        let rate =
            toNumber(flow[1].rate_estimate_bps.down, 0) +
            toNumber(flow[1].rate_estimate_bps.up, 0);
        if (prevFlowBytes.has(flowKey)) {
            let down = diffToNumber(flow[1].bytes_sent.down, prevFlowBytes.get(flowKey)[0], 0);
            let up = diffToNumber(flow[1].bytes_sent.up, prevFlowBytes.get(flowKey)[1], 0);
            rate = down + up;
        }
        if (movingAverages.has(flowKey)) {
            let avg = movingAverages.get(flowKey);
            avg.push(rate);
            if (avg.length > 10) {
                avg.shift();
            }
            movingAverages.set(flowKey, avg);
        } else {
            movingAverages.set(flowKey, [ rate ]);
        }
    });

    // Process flows and collect data
    msg.flows.forEach((flow) => {
        let flowKey = flow[0].protocol_name + flow[0].row_id;
        let down = toNumber(flow[1].rate_estimate_bps.down, 0);
        let up = toNumber(flow[1].rate_estimate_bps.up, 0);

        //console.log(flow);
        if (prevFlowBytes.has(flowKey)) {
            let prev = prevFlowBytes.get(flowKey);
            let ticks = tickCount - prev[2];
            if (ticks === 1) {
                down = diffToNumber(flow[1].bytes_sent.down, prev[0], 0) * 8;
                up = diffToNumber(flow[1].bytes_sent.up, prev[1], 0) * 8;
            } else if (ticks > 1) {
                down = diffToNumber(flow[1].bytes_sent.down, prev[0], 0) * 8;
                up = diffToNumber(flow[1].bytes_sent.up, prev[1], 0) * 8;
                down = down / ticks;
                up = up / ticks;
            }
        }
        if (down < 0) down = 0;
        if (up < 0) up = 0;
        prevFlowBytes.set(flowKey, [ flow[1].bytes_sent.down, flow[1].bytes_sent.up, tickCount ]);

        const lastSeenNanos = toNumber(flow[0].last_seen_nanos, 0);
        if (lastSeenNanos > thirty_seconds_in_nanos) return;
        
        let opacity = Math.min(1, lastSeenNanos / thirty_seconds_in_nanos);
        let visible = !hideSmallFlows || (down > 1024*1024 || up > 1024*1024);
        
        // Calculate retransmit percentages
        let retransmitDown = "-";
        let retransmitUp = "-";
        let retransmitDownPct = 0;
        let retransmitUpPct = 0;
        
        const tcpRetransmitsDown = toNumber(flow[1].tcp_retransmits.down, 0);
        const tcpRetransmitsUp = toNumber(flow[1].tcp_retransmits.up, 0);
        const packetsSentDown = toNumber(flow[1].packets_sent.down, 0);
        const packetsSentUp = toNumber(flow[1].packets_sent.up, 0);

        if (tcpRetransmitsDown > 0 && packetsSentDown > 0) {
            retransmitDownPct = tcpRetransmitsDown / packetsSentDown;
            retransmitDown = formatRetransmit(retransmitDownPct);
        }
        if (tcpRetransmitsUp > 0 && packetsSentUp > 0) {
            retransmitUpPct = tcpRetransmitsUp / packetsSentUp;
            retransmitUp = formatRetransmit(retransmitUpPct);
        }
        
        // Get average rate for sorting
        let avgRate = down + up;
        if (movingAverages.has(flowKey)) {
            const avg = movingAverages.get(flowKey);
            avgRate = avg.reduce((a, b) => a + b, 0) / avg.length;
        }
        
        const bytesSentDown = toNumber(flow[1].bytes_sent.down, 0);
        const bytesSentUp = toNumber(flow[1].bytes_sent.up, 0);
        const rttDownNanos = toNumber(flow[1].rtt[0].nanoseconds, 0);
        const rttUpNanos = toNumber(flow[1].rtt[1].nanoseconds, 0);

        const qoq = flow[1].qoq || null;
        const qooDown = qoq ? qoq.download_total : null;
        const qooUp = qoq ? qoq.upload_total : null;
        const qooForSort = (typeof qooDown === "number" ? qooDown : 0) + (typeof qooUp === "number" ? qooUp : 0);

        // Collect row data
        tableRows.push({
            sortKeys: {
                protocol: flow[0].protocol_name,
                rate: avgRate,
                bytes: bytesSentDown + bytesSentUp,
                packets: packetsSentDown + packetsSentUp,
                retransmits: retransmitDownPct + retransmitUpPct,
                rtt: rttDownNanos + rttUpNanos,
                qoo: qooForSort,
                asn: flow[0].asn_name || "",
                country: flow[0].asn_country || "",
                ip: flow[0].remote_ip
            },
            columns: [
                flow[0].protocol_name,
                formatThroughput(down, plan.down),
                formatThroughput(up, plan.up),
                scaleNumber(bytesSentDown),
                scaleNumber(bytesSentUp),
                scaleNumber(packetsSentDown),
                scaleNumber(packetsSentUp),
                retransmitDown,
                retransmitUp,
                formatRttNanos(rttDownNanos),
                formatRttNanos(rttUpNanos),
                formatQooScore(qooDown),
                formatQooScore(qooUp),
                flow[0].asn_name,
                flow[0].asn_country,
                flow[0].remote_ip
            ],
            opacity: 1.0 - opacity,
            visible: visible
        });
    });
    
    // Sort tableRows based on current sort preferences
    tableRows.sort((a, b) => {
        let aVal = a.sortKeys[trafficSortColumn];
        let bVal = b.sortKeys[trafficSortColumn];
        
        // Handle string vs number comparison
        if (typeof aVal === 'string' && typeof bVal === 'string') {
            aVal = aVal.toLowerCase();
            bVal = bVal.toLowerCase();
        }
        
        if (trafficSortDirection === 'asc') {
            return aVal < bVal ? -1 : aVal > bVal ? 1 : 0;
        } else {
            return aVal > bVal ? -1 : aVal < bVal ? 1 : 0;
        }
    });
    
    // Render the sorted table
    tableRows.forEach((rowData) => {
        if (!rowData.visible) return;
        
        let row = document.createElement("tr");
        row.classList.add("small");
        row.style.opacity = rowData.opacity;
        
        // Add columns
        rowData.columns.forEach((col, index) => {
            if (index === 1 || index === 2 || index === 7 || index === 8 || index === 9 || index === 10 || index === 11 || index === 12) {
                // These columns have HTML formatting
                row.appendChild(simpleRowHtml(col));
            } else {
                row.appendChild(simpleRow(col));
            }
        });
        
        tbody.appendChild(row);
    });

    table.appendChild(tbody);

    clearDiv(target);
    target.appendChild(table);
}

function updateSpeedometer(devices) {
    let totalDown = 0;
    let totalUp = 0;
    let planDown = 0;
    let planUp = 0;
    let retransmitsDown = 0;
    let retransmitsUp = 0;
    devices.forEach((device) => {
        const deviceDown = toNumber(device.bytes_per_second.down, 0);
        const deviceUp = toNumber(device.bytes_per_second.up, 0);
        totalDown += deviceDown;
        totalUp += deviceUp;
        planDown = Math.max(planDown, toNumber(device.plan.down, 0));
        planUp = Math.max(planUp, toNumber(device.plan.up, 0));
        retransmitsDown += toNumber(device.tcp_retransmits.down, 0);
        retransmitsUp += toNumber(device.tcp_retransmits.up, 0);

        let throughputGraph = deviceGraphs["throughputGraph_" + device.device_id];
        if (throughputGraph !== undefined) {
            throughputGraph.update(deviceDown * 8, deviceUp * 8);
        }

        let retransmitGraph = deviceGraphs["tcpRetransmitsGraph_" + device.device_id];
        if (retransmitGraph !== undefined) {
            retransmitGraph.update(
                toNumber(device.tcp_retransmits.down, 0),
                toNumber(device.tcp_retransmits.up, 0)
            );
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
            throughputDown.innerHTML = formatThroughput(
                toNumber(device.bytes_per_second.down, 0) * 8,
                toNumber(device.plan.down, 0)
            );
        }

        if (throughputUp !== null) {
            throughputUp.innerHTML = formatThroughput(
                toNumber(device.bytes_per_second.up, 0) * 8,
                toNumber(device.plan.up, 0)
            );
        }

        if (rttDown !== null) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const curP95 = device.rtt_current_p95_nanos || {};
            const totP50 = device.rtt_total_p50_nanos || {};
            const totP95 = device.rtt_total_p95_nanos || {};
            rttDown.innerHTML =
                "<div class='tiny'>C: " +
                formatRttPair(curP50.down, curP95.down) +
                "</div><div class='tiny text-secondary'>T: " +
                formatRttPair(totP50.down, totP95.down) +
                "</div>";
        }

        if (rttUp !== null) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const curP95 = device.rtt_current_p95_nanos || {};
            const totP50 = device.rtt_total_p50_nanos || {};
            const totP95 = device.rtt_total_p95_nanos || {};
            rttUp.innerHTML =
                "<div class='tiny'>C: " +
                formatRttPair(curP50.up, curP95.up) +
                "</div><div class='tiny text-secondary'>T: " +
                formatRttPair(totP50.up, totP95.up) +
                "</div>";
        }

        if (tcp_retransmitsDown !== null) {
            tcp_retransmitsDown.innerHTML = formatRetransmit(device.tcp_retransmits.down);
        }

        if (tcp_retransmitsUp !== null) {
            tcp_retransmitsUp.innerHTML = formatRetransmit(device.tcp_retransmits.up);
        }

        // Local RTT histogram (5-minute window, p50 samples)
        let rttHistogram = deviceGraphs["rttHistogramGraph_" + device.device_id];
        if (rttHistogram !== undefined) {
            const curP50 = device.rtt_current_p50_nanos || {};
            const downNanos = toNumber(curP50.down, 0);
            const upNanos = toNumber(curP50.up, 0);
            const samples = [];
            if (downNanos > 0) samples.push(downNanos / 1000000);
            if (upNanos > 0) samples.push(upNanos / 1000000);
            rttHistogram.updateManyMs(samples);
        }
    });
}

function initialDevices(circuits) {
    let target = document.getElementById("devices");
    clearDiv(target);

    circuits.forEach((circuit) => {
        let outer = document.createElement("div");
        outer.classList.add("col-12", "mb-3");
        target.appendChild(outer);

        let row = document.createElement("div");
        row.classList.add("row", "g-2");
        outer.appendChild(row);

        let d = document.createElement("div");
        d.classList.add("col-3");
        row.appendChild(d);

        // Device Information Section

        let name = document.createElement("h5");
        name.classList.add("redactable");
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
        td.classList.add("redactable");
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
        let ipv4Table = document.createElement("table");
        ipv4Table.classList.add("table", "table-sm");
        let ipv4Body = document.createElement("tbody");
        circuit.ipv4.forEach((ip) => {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = ip[0] + "/" + ip[1];
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ip[0];
            value.innerText = "-";
            tr.appendChild(value);
            ipv4Body.appendChild(tr);
        });
        if (circuit.ipv4.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv4 addresses assigned";
            tr.appendChild(label);
            ipv4Body.appendChild(tr);
        }
        ipv4Table.appendChild(ipv4Body);
        td.appendChild(ipv4Table);

        tr3.appendChild(td);
        tbody.appendChild(tr3);

        // IPv6 Row
        let tr4 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>IPv6 Address(es)</b>";
        tr4.appendChild(td);
        td = document.createElement("td");
        td.colSpan = 2;

        let ipv6 = document.createElement("table");
        ipv6.classList.add("table", "table-sm");
        let ipv6Body = document.createElement("tbody");
        circuit.ipv6.forEach((ip) => {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = ip[0] + "/" + ip[1];
            label.classList.add("small");
            tr.appendChild(label);
            let value = document.createElement("td");
            value.id = "ip_" + ip[0];
            value.innerText = "-";
            tr.appendChild(value);
            ipv6Body.appendChild(tr);
        });
        if (circuit.ipv6.length === 0) {
            let tr = document.createElement("tr");
            let label = document.createElement("td");
            label.innerHTML = "No IPv6 addresses assigned";
            tr.appendChild(label);
            ipv6Body.appendChild(tr);
        }
        ipv6.appendChild(ipv6Body);
        td.appendChild(ipv6);

        /*let ipv6 = "";
        circuit.ipv6.forEach((ip) => {
            ipv6 += ip[0] + "/" + ip[1] + "<br>";
        });
        if (ipv6 === "") ipv6 = "No IPv6 addresses assigned";
        td.innerHTML = ipv6;*/
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
        td.innerHTML = "<b>RTT P50/P95</b>";
        tr6.appendChild(td);
        td = document.createElement("td");
        td.id = "rttDown_" + circuit.device_id;
        td.innerHTML = "<span class='text-secondary'>Sampling...</span>";
        tr6.appendChild(td);
        td = document.createElement("td");
        td.id = "rttUp_" + circuit.device_id;
        td.innerHTML = "<span class='text-secondary'>Sampling...</span>";
        tr6.appendChild(td);
        tbody.appendChild(tr6);

        // Placeholder for TCP Retransmits
        let tr7 = document.createElement("tr");
        td = document.createElement("td");
        td.innerHTML = "<b>TCP Re-Xmits</b>";
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

        // Graph container (2x2)
        let graphCol = document.createElement("div");
        graphCol.classList.add("col-9");
        row.appendChild(graphCol);

        let graphRow = document.createElement("div");
        graphRow.classList.add("row", "g-2");
        graphCol.appendChild(graphRow);

        function addGraph(divId, graphFactory) {
            let col = document.createElement("div");
            col.classList.add("col-6");
            let div = document.createElement("div");
            div.id = divId;
            div.style.height = "250px";
            div.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Loading...";
            col.appendChild(div);
            graphRow.appendChild(col);
            deviceGraphs[divId] = graphFactory(divId);
        }

        addGraph("throughputGraph_" + circuit.device_id, (id) => new CircuitTotalGraph(id, "Throughput"));
        addGraph("tcpRetransmitsGraph_" + circuit.device_id, (id) => new CircuitRetransmitGraph(id, "Retransmits"));
        addGraph("rttHistogramGraph_" + circuit.device_id, (id) => new WindowedLatencyHistogram(id, "RTT Histogram", 300000));
        addGraph("pingGraph_" + circuit.device_id, (id) => new DevicePingHistogram(id));

    });
}

function initialFunnel(parentNode) {
    let target = document.getElementById("theFunnel");
    listenOnce("NetworkTree", (msg) => {
        const data = msg && msg.data ? msg.data : [];
        let immediateParent = null;
        data.forEach((node) => {
            if (node[1].name === parentNode) {
                immediateParent = node[1];
            }
        });

        if (immediateParent === null) {
            clearDiv(target);
            target.appendChild(document.createTextNode("No parent node found"));
            return;
        }

        let parentDiv = document.createElement("div");
        immediateParent.parents.reverse().forEach((parent) => {
            //console.log(data[parent]);
            let row = document.createElement("div");
            row.classList.add("row");

            let col = document.createElement("div");
            col.classList.add("col-12");
            let heading = document.createElement("h5");
            heading.classList.add("redactable");
            heading.innerHTML = "<i class='fa fa-sitemap'></i> " + data[parent][1].name;
            col.appendChild(heading);
            row.appendChild(col);
            parentDiv.appendChild(row);

            // Row for graphs
            row = document.createElement("div");
            row.classList.add("row");

            let col_tp = document.createElement("div");
            col_tp.classList.add("col-4");
            col_tp.id = "funnel_tp_" + parent;
            col_tp.style.height = "250px";
            row.appendChild(col_tp);

            let col_rxmit = document.createElement("div");
            col_rxmit.classList.add("col-4");
            col_rxmit.id = "funnel_rxmit_" + parent;
            row.appendChild(col_rxmit);

            let col_rtt = document.createElement("div");
            col_rtt.classList.add("col-4");
            col_rtt.id = "funnel_rtt_" + parent;
            row.appendChild(col_rtt);

            parentDiv.appendChild(row);
        });
        clearDiv(target);
        target.appendChild(parentDiv);
        // Ugly hack to defer until the DOM is updated
        requestAnimationFrame(() => {setTimeout(() => {
            immediateParent.parents.reverse().forEach((parent) => {
                let tpGraph = new CircuitTotalGraph("funnel_tp_" + parent, "Throughput");
                let rxmitGraph = new CircuitRetransmitGraph("funnel_rxmit_" + parent, "Retransmits");
                let rttGraph = new WindowedLatencyHistogram("funnel_rtt_" + parent, "Latency Histogram", 300000);
                funnelGraphs[parent] = {
                    tp: tpGraph,
                    rxmit: rxmitGraph,
                    rtt: rttGraph,
                };
            });
            funnelParents = immediateParent.parents;
            subscribeWS(["NetworkTree"], onTreeEvent);
        }, 0)});
    });
    wsClient.send({ NetworkTree: {} });
}

function onTreeEvent(msg) {
    //console.log(msg);
    funnelParents.forEach((parent) => {
        if (msg.event !== "NetworkTree") return;
        let myMessage = msg.data[parent][1];
        if (myMessage === undefined) return;
        let tpGraph = funnelGraphs[parent].tp;
        let rxmitGraph = funnelGraphs[parent].rxmit;
        let rttGraph = funnelGraphs[parent].rtt;

        tpGraph.update(
            toNumber(myMessage.current_throughput[0], 0) * 8,
            toNumber(myMessage.current_throughput[1], 0) * 8
        );
        let rxmit = [0, 0];
        const packetsDown = toNumber(myMessage.current_tcp_packets[0], 0);
        const packetsUp = toNumber(myMessage.current_tcp_packets[1], 0);
        const retransmitsDown = toNumber(myMessage.current_retransmits[0], 0);
        const retransmitsUp = toNumber(myMessage.current_retransmits[1], 0);
        if (retransmitsDown > 0 && packetsDown > 0) {
            rxmit[0] = (retransmitsDown / packetsDown) * 100.0;
        }
        if (retransmitsUp > 0 && packetsUp > 0) {
            rxmit[1] = (retransmitsUp / packetsUp) * 100.0;
        }
        rxmitGraph.update(rxmit[0], rxmit[1]);
        rttGraph.updateManyMs(myMessage.rtts);
        tpGraph.chart.resize();
        rxmitGraph.chart.resize();
        rttGraph.chart.resize();
    });
}

function subscribeToCake() {
    let backlogGraph = new CakeBacklog("cakeBacklog");
    let delaysGraph = new CakeDelays("cakeDelays");
    let queueLength = new CakeQueueLength("cakeQueueLength");
    let traffic = new CakeTraffic("cakeTraffic");
    let marks = new CakeMarks("cakeMarks");
    let drops = new CakeDrops("cakeDrops");
    let noDataTimeout = null;
    let hasReceivedData = false;
    
    // Function to show "Queue not loaded" message
    function showNoQueueMessage() {
        const cakeTab = document.getElementById("cake");
        if (cakeTab && !hasReceivedData) {
            cakeTab.innerHTML = '<div class="row"><div class="col-12 text-center mt-5"><h4>Queue not loaded.</h4><p class="text-muted">The shaper queue for this circuit has not been created yet.</p></div></div>';
        }
    }
    
    // Set a timeout to show the message if no data arrives within 3 seconds
    noDataTimeout = setTimeout(showNoQueueMessage, 3000);
    
    channelLink = new DirectChannel({
        CakeWatcher: {
            circuit: circuit_id
        }
    }, (msg) => {
        //console.log(msg);
        
        // Clear the timeout and set flag that we've received data
        if (noDataTimeout) {
            clearTimeout(noDataTimeout);
            noDataTimeout = null;
        }
        
        // If this is the first data received, restore the original HTML structure
        if (!hasReceivedData) {
            hasReceivedData = true;
            // Update the tab heading based on queue kind
            try {
                const kindDown = (msg.kind_down || '').toLowerCase();
                const kindUp = (msg.kind_up || '').toLowerCase();
                const tabBtn = document.getElementById('cake-tab');
                const tabLi = tabBtn ? tabBtn.parentElement : null;
                if (kindDown === 'none' && kindUp === 'none') {
                    // Hide the shaper overview tab entirely for SQM=none
                    if (tabLi) tabLi.style.display = 'none';
                    const tabContent = document.getElementById('cake');
                    if (tabContent) tabContent.style.display = 'none';
                    return; // Skip building graphs
                } else {
                    let displayKind = 'Shaper Overview';
                    if (kindDown === 'cake' || kindUp === 'cake') {
                        displayKind = 'CAKE Shaper Overview';
                    } else if (kindDown === 'fq_codel' || kindUp === 'fq_codel') {
                        displayKind = 'fq_codel Shaper Overview';
                    }
                    if (tabBtn) {
                        tabBtn.innerHTML = '<i class="fa fa-birthday-cake"></i> ' + displayKind;
                    }
                }
            } catch (e) { /* ignore */ }
            const cakeTab = document.getElementById("cake");
            if (cakeTab) {
                cakeTab.innerHTML = `
                    <div class="row">
                        <div class="col-4">
                            <div id="cakeBacklog" style="height: 250px"></div>
                        </div>
                        <div class="col-4">
                            <div id="cakeDelays" style="height: 250px"></div>
                        </div>
                        <div class="col-4">
                            <div id="cakeQueueLength" style="height: 250px"></div>
                        </div>
                        <div class="col-4">
                            <div id="cakeTraffic" style="height: 250px"></div>
                        </div>
                        <div class="col-4">
                            <div id="cakeMarks" style="height: 250px"></div>
                        </div>
                        <div class="col-4">
                            <div id="cakeDrops" style="height: 250px"></div>
                        </div>
                        <div class="col-3">
                            Queue Memory: <span id="cakeQueueMemory">?</span>
                        </div>
                    </div>
                `;
                // Reinitialize the graphs
                backlogGraph = new CakeBacklog("cakeBacklog");
                delaysGraph = new CakeDelays("cakeDelays");
                queueLength = new CakeQueueLength("cakeQueueLength");
                traffic = new CakeTraffic("cakeTraffic");
                marks = new CakeMarks("cakeMarks");
                drops = new CakeDrops("cakeDrops");
            }
        }

        // Cake Memory Usage
        $("#cakeQueueMemory").text(scaleNumber(msg.current_download.memory_used) + " / " + scaleNumber(msg.current_upload.memory_used));
        backlogGraph.update(msg);
        backlogGraph.chart.resize();
        delaysGraph.update(msg);
        delaysGraph.chart.resize();
        queueLength.update(msg);
        queueLength.chart.resize();
        traffic.update(msg);
        traffic.chart.resize();
        marks.update(msg);
        marks.chart.resize();
        drops.update(msg);
        drops.chart.resize();
    });
}

function wireupAnalysis(circuits) {
    let ipAddresses = fullIpList(circuits);
    let list = document.createElement("div");
    let listBtn = document.createElement("button");
    listBtn.type = "button";
    listBtn.id = "CaptureTopBtn";
    listBtn.classList.add("btn", "btn-secondary", "dropdown-toggle", "btn-sm");
    listBtn.setAttribute("data-bs-toggle", "dropdown");
    listBtn.innerHTML = "<i class='fa fa-search'></i> Packet Capture";
    list.appendChild(listBtn);

    let listUl = document.createElement("ul");
    listUl.classList.add("dropdown-menu", "dropdown-menu-sized");
    ipAddresses.forEach((ip) => {
        let entry = document.createElement("li");
        let item = document.createElement("a");
        item.classList.add("dropdown-item");
        item.innerHTML = "<i class='fa fa-search'></i> Capture packets from " + ip[0];
        let address = ip[0]; // For closure capture
        item.onclick = () => {
            //console.log("Clicky " + address);
            listenOnce("RequestAnalysisResult", (msg) => {
                const data = msg ? msg.data : null;
                const okData = data && data.Ok ? data.Ok : null;
                if (!okData) {
                    alert("Packet capture is already active for another IP. Please try again when it is finished.")
                    return;
                }
                let counter = parseInt(okData.countdown) + 1;
                let sessionId = okData.session_id;
                let btn = document.getElementById("CaptureTopBtn");
                btn.disabled = true;
                btn.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Capturing Packets (" + counter + ")";
                let interval = setInterval(() => {
                    counter--;
                    if (counter === -1) {
                        clearInterval(interval);
                        btn.disabled = false;
                        btn.innerHTML = "<i class='fa fa-download'></i> Download Packet Capture for " + address;
                        btn.classList.remove("btn-secondary");
                        btn.classList.add("btn-success");
                        btn.onclick = () => {
                            let url = "/local-api/pcapDump/" + sessionId;
                            download(url, "capture.pcap");
                            //console.log(url);

                            // Restore the buttons
                            requestCircuitById((circuits) => {
                                wireupAnalysis(circuits);
                            });
                        }
                        return;
                    }
                    btn.innerHTML = "<i class='fa fa-spinner fa-spin'></i> Capturing Packets (" + counter + ")";
                }, 1000);
            });
            wsClient.send({ RequestAnalysis: { ip: address } });
        }
        entry.appendChild(item);
        listUl.appendChild(entry);
    });
    list.appendChild(listUl);
    let parent = document.getElementById("captureButton");
    clearDiv(parent);
    parent.appendChild(list);
}

function download(dataurl, filename) {
    const link = document.createElement("a");
    link.href = dataurl;
    link.download = filename;
    link.click();
}

function loadInitial() {
    requestCircuitById((circuits) => {
        let circuit = circuits[0];
        $("#circuitName").text(circuit.circuit_name);
        $("#parentNode").text(circuit.parent_node);
        $("#bwMax").text(formatMbps(circuit.download_max_mbps) + " / " + formatMbps(circuit.upload_max_mbps));
        $("#bwMin").text(formatMbps(circuit.download_min_mbps) + " / " + formatMbps(circuit.upload_min_mbps));
        plan = {
            down: toNumber(circuit.download_max_mbps, 0),
            up: toNumber(circuit.upload_max_mbps, 0),
        };
        initialDevices(circuits);
        speedometer = new BitsPerSecondGauge("bitsGauge", "Plan");
        qooGauge = new QooScoreGauge("qooGauge");
        totalThroughput = new CircuitTotalGraph("throughputGraph", "Total Circuit Throughput");
        totalRetransmits = new CircuitRetransmitGraph("rxmitGraph", "Total Circuit Retransmits");
        flowSankey = new FlowsSankey("flowSankey");

        connectPrivateChannel();
        connectPingers(circuits);
        connectFlowChannel();
        initialFunnel(circuit.parent_node);
        subscribeToCake();
        wireupAnalysis(circuits);
    }, () => {
        alert("Circuit with id " + circuit_id + " not found");
    });
}

loadInitial();
