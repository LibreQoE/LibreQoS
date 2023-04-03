function bytesToSize(bytes) {
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    let i = parseInt(Math.floor(Math.log(bytes) / Math.log(1024)));
    if (i == 0) return bytes + ' ' + sizes[i];
    return (bytes / Math.pow(1024, i)).toFixed(1) + ' ' + sizes[i];
}

function bitsToSpeed(bits) {
    const speeds = ['bps', 'Kbps', 'Mbps', 'Gbps', 'Tbps'];
    if (bits == 0)
        return '0bps';
    let i = parseInt(Math.floor(Math.log(bits) / Math.log(1000)));
    if (i == 0) return bits + ' ' + speeds[i];
    return (bits / Math.pow(1000, i)).toFixed(1) + '' + speeds[i];
}

// Need this to send a websocket message and store the users browser state there.
function toggleThemeChange(src) {
	var event = document.createEvent('Event');
	if (document.body.classList.contains('dark-theme')) {
		document.body.classList.remove('dark-theme');
	} else {
		document.body.classList.add('dark-theme');
	}
	document.body.dispatchEvent(event);
}

const rampColors = {
    meta: ["#32b08c", "#ffb94a", "#f95f53", "#bf3d5e", "#dc4e58"],
    regular: ["#aaffaa", "goldenrod", "#ffaaaa"],
};

function scaleNumber(n) {
    if (n > 1000000000000) {
        return (n/1000000000000).toFixed(2) + "T";
    } else if (n > 1000000000) {
        return (n/1000000000).toFixed(2) + "G";
    } else if (n > 1000000) {
        return (n/1000000).toFixed(2) + "M";
    } else if (n > 1000) {
        return (n/1000).toFixed(2) + "K";
    }
    return n;
}

class LqosWs {
    document;
    socketRef;
    eventHandler;
    #DataBuffer = class {
        #data;
        #Ring = class {
            constructor(structureFn) {
                structureFn(this);
            }
        }
        constructor() {
    
        }
        push(id, data, shift = false) {
            if (!this.data.hasOwnProperty(id))
                this.#data[id] = new this.#Ring(data);
            if (shift)
                this.#data[id].shift();
            this.data[id].push(data);
        }
        clear(id) {
    
        }
    }
    #Events = class {
        document;
        socketRef;
        #key_function_map = {
            cpu: this.#cpu,
            disk: this.#disk,
            ram: this.#ram,
            shaped_count: this.#shaped_count,
            unknown_count: this.#unknown_count,
            rtt: this.#rtt,
            current_throughput: this.#current_throughput,
            site_funnel: this.#site_funnel,
            top_ten_download: this.#top_ten_download,
            worst_ten_rtt: this.#worst_ten_rtt,
        }
        constructor(lqos_ws) {
            this.document = lqos_ws.document;
            this.socketRef = lqos_ws.socketRef;
        }
        dispatch(message) {
            console.log("dispatching message: {}", message);
            this.document.dispatchEvent(new CustomEvent(message.subject, { detail: message }));
        }
        process(event) {
            this.#key_function_map[event.detail.subject](JSON.parse(event.detail.data));
        }
        reload_lqos() {

        }
        subscribe(data) {
            let event = this;
            this.document.addEventListener(data.data.subject, function(e){event.process(e)});
            this.socketRef.send(JSON.stringify(data));
        }
        #cpu(data) {
            var cpu_sum = data.reduce((partialSum, a) => partialSum + a, 0);
            var cpu_avg = Math.ceil(cpu_sum/data.length);
            document.querySelectorAll("[data-lqos-cpu]").forEach((span) => {
                span.style.width = cpu_avg + "%";
            });
        }
        #current_throughput(data) {
            if (!this.ctGraph)
                this.ctGraph = new MultiRingBuffer(300);
            document.querySelector('.live-download-packets').innerText = scaleNumber(data['packets_per_second'][0]);
            document.querySelector('.live-upload-packets').innerText = scaleNumber(data['packets_per_second'][1]);
            document.querySelector('.live-shaped-download').innerText = bitsToSpeed(data['shaped_bits_per_second'][0]);
            let unshaped_bits_download = data['bits_per_second'][0] - data['shaped_bits_per_second'][0];
            document.querySelector('.live-unshaped-download').innerText = bitsToSpeed(unshaped_bits_download);
            document.querySelector('.live-shaped-upload').innerText = bitsToSpeed(data['shaped_bits_per_second'][1]);
            let unshaped_bits_upload = data['bits_per_second'][1] - data['shaped_bits_per_second'][1];
            document.querySelector('.live-unshaped-upload').innerText = bitsToSpeed(unshaped_bits_upload);
            document.querySelector('.live-download').innerText = bitsToSpeed(data['bits_per_second'][0]);
            document.querySelector('.live-upload').innerText = bitsToSpeed(data['bits_per_second'][1]);
            this.ctGraph.push("pps", data['packets_per_second'][0], data['packets_per_second'][1]);
            this.ctGraph.push("total", data['bits_per_second'][0], data['bits_per_second'][1]);
            this.ctGraph.push("shaped", data['shaped_bits_per_second'][0], data['shaped_bits_per_second'][1]);
            this.ctGraph.plotTotalThroughput(document.querySelector('[data-lqos-subscribe="current_throughput"]'));
        }
        #disk(data) {
            document.querySelectorAll('[data-lqos-disk]').forEach((span) => {
                span.innerHTML = data;
            });
        }
        #ram(data) {
            let consumed_ram = data[0];
            let total_ram = data[1];
            let ram_percentage = Math.ceil(consumed_ram/total_ram);
            document.querySelectorAll('[data-lqos-ram-consumed]').forEach((span) => {
                span.innerText = bytesToSize(consumed_ram);
            });
            document.querySelectorAll('[data-lqos-ram-total]').forEach((span) => {
                span.innerText = bytesToSize(total_ram);
            });
            document.querySelectorAll('[data-lqos-ram]').forEach((span) => {
                span.style.width = ram_percentage + "%";
            });
        }
        #rtt(data) {
            if (!this.rttGraph)
                this.rttGraph = new RttHistogram();
            this.rttGraph.clear();
            for (let i = 0; i < data.length; i++) {
                this.rttGraph.pushBand(i, data[i]);
            }
            this.rttGraph.plot(document.querySelector('[data-lqos-subscribe="rtt"]'));
        }
        #shaped_count(data) {
            document.querySelectorAll('[data-lqos-shaped-count]').forEach((span) => {
                span.innerHTML = data;
            });
        }
        #site_funnel(data) {
            let ntbody = document.createElement("tbody");
            ntbody.setAttribute("id", "live-site-funnel");
            data.forEach((circuit) => {
                let tr = document.createElement("tr");
                let c1 = document.createElement("td");
                c1.classList.add("text-truncate");
                c1.setAttribute("width", "40%");
                c1.innerHTML = "<a href=\"/tree/"+ circuit[0] +"\">" + circuit[1].name + "</a>";
                tr.appendChild(c1);
                let c2 = document.createElement("td");
                c2.setAttribute("width", "30%");
                let down_scale = ((circuit[1].current_throughput[0] / 100000) / circuit[1].max_throughput[0]) * 200;
                if (circuit[1].name == "Others")
                    c2.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: #808080;\"></i>";
                else
                    c2.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: "+color_ramp(down_scale)+";\"></i>";
                c2.innerHTML += bitsToSpeed(circuit[1].current_throughput[0] * 8);
                tr.appendChild(c2);
                let c3 = document.createElement("td");
                c3.setAttribute("width", "30%");
                let up_scale = ((circuit[1].current_throughput[1] / 100000) / circuit[1].max_throughput[1]) * 200;
                if (circuit[1].name == "Others")
                    c3.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: #808080;\"></i>";
                else
                    c3.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: "+color_ramp(up_scale)+";\"></i>";
                c3.innerHTML += bitsToSpeed(circuit[1].current_throughput[1] * 8);
                tr.appendChild(c3);
                ntbody.appendChild(tr);
            });
            let otbody = document.getElementById("live-site-funnel");
            otbody.parentNode.replaceChild(ntbody, otbody);
        }
        #top_ten_download(data) {
            let ntbody = document.createElement("tbody");
            ntbody.setAttribute("id", "live-tt");
            data.forEach((circuit) => {
                let tr = document.createElement("tr");
                let c1 = document.createElement("td");
                c1.classList.add("text-truncate");
                if (circuit.circuit_id)
                    c1.innerHTML = "<a href=\"/circuit/"+ circuit.circuit_id +"\">" + circuit.ip_address + "</a>";
                else
                    c1.innerText = circuit.ip_address;
                tr.appendChild(c1);
                let c2 = document.createElement("td");
                let down_scale = (circuit.bits_per_second[0] / (circuit.plan[0] * 1000000)) * 200;
                c2.innerHTML = bitsToSpeed(circuit.bits_per_second[0]);
                tr.appendChild(c2);
                let c3 = document.createElement("td");
                let up_scale = (circuit.bits_per_second[1] / (circuit.plan[1] * 1000000)) * 200;
                c3.innerHTML = bitsToSpeed(circuit.bits_per_second[1]);
                tr.appendChild(c3);
                let c4 = document.createElement("td");
                c4.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: "+color_ramp(circuit.median_tcp_rtt)+";\"></i>";
                c4.innerHTML += circuit.median_tcp_rtt.toFixed(2);
                tr.appendChild(c4);
                let c5 = document.createElement("td");
                if (circuit.tc_handle != 0) {
                    c5.innerHTML = "<i class=\"fa fa-circle-check text-success fa-fw pe-2\"></i>";
                    c5.innerHTML += circuit.plan[0] + "/" + circuit.plan[1];
                } else {
                    c5.innerHTML = "<i class=\"fa fa-circle-xmark text-danger fa-fw pe-2\"></i>";
                    c5.innerHTML += "N/A";
                }
                tr.appendChild(c5);
                ntbody.appendChild(tr);
            });
            let otbody = document.getElementById("live-tt");
            otbody.parentNode.replaceChild(ntbody, otbody);
        }
        #worst_ten_rtt(data) {
            const ntbody = document.createElement("tbody");
            ntbody.setAttribute("id", "live-wrtt");
            data.forEach((circuit) => {
                let tr = document.createElement("tr");
                let c1 = document.createElement("td");
                c1.classList.add("text-truncate");
                if (circuit.circuit_id)
                    c1.innerHTML = "<a href=\"/circuit/"+ circuit.circuit_id +"\">" + circuit.ip_address + "</a>";
                else
                    c1.innerText = circuit.ip_address;
                tr.appendChild(c1);
                let c2 = document.createElement("td");
                c2.innerHTML = bitsToSpeed(circuit.bits_per_second[0]);
                tr.appendChild(c2);
                let c3 = document.createElement("td");
                c3.innerHTML = bitsToSpeed(circuit.bits_per_second[1]);
                tr.appendChild(c3);
                let c4 = document.createElement("td");
                c4.innerHTML = "<i class=\"fa fa-square fa-fw pe-2\" style=\"color: "+color_ramp(circuit.median_tcp_rtt)+";\"></i>";
                c4.innerHTML += circuit.median_tcp_rtt.toFixed(2);
                tr.appendChild(c4);
                let c5 = document.createElement("td");
                if (circuit.tc_handle != 0) {
                    c5.innerHTML = "<i class=\"fa fa-circle-check text-success fa-fw pe-2\"></i>";
                    c5.innerHTML += circuit.plan[0] + "/" + circuit.plan[1];
                } else {
                    c5.innerHTML = "<i class=\"fa fa-circle-xmark text-danger fa-fw pe-2\"></i>";
                    c5.innerHTML += "N/A";
                }
                tr.appendChild(c5);
                ntbody.appendChild(tr);
            });
            let otbody = document.getElementById("live-wrtt");
            otbody.parentNode.replaceChild(ntbody, otbody);
        }
        #unknown_count(data) {
            let spans = document.querySelectorAll('[data-lqos-unknown-count]').forEach((span) => {
                span.innerHTML = data;
            });
        }
    }
    #Message = class Message {
        data;
        subject;
        packed = false;
        #subject_keys = {
            // Add keys here for validation
            cpu: "cpu",
            current_throughput: "current_throughput",
            disk: "disk",
            ram: "ram",
            rtt: "rtt",
            shaped_count: "shaped_count",
            site_funnel: "site_funnel",
            top_ten_download: "top_ten_download",
            unknown_count: "unknown_count",
            worst_ten_rtt: "worst_ten_rtt",
        }
        constructor(event) {
            this.subject = event.subject;
            this.data = event.data;
            (event.packed) && this.#decode();
        }
        #decode() {
            this.data = msgpack.decode(new Uint8Array(this.data));
        }
        toJSON() {
            return {
                subject: this.subject,
                data: this.data,
                packed: this.packed
            }
        }
        process(eventHandler) {
            if (this.subject && this.#subject_keys[this.subject]){
                eventHandler.dispatch(this);
            } else {
                if (this.subject == "subscribe") {
                    if (this.data && this.#subject_keys[this.data.subject])
                        eventHandler.subscribe(this);
                } else if (this.subject == "unsubscribe") {
                    if (this.data && this.#subject_keys[this.data.subject])
                        eventHandler.unsubscribe(this);
                }
            }
        }
    }
    #Connect() {
        this.socketRef = new WebSocket('ws://' + location.host + '/ws');
        let lqos_ws = this;
        this.socketRef.onopen = function(){
            lqos_ws.eventHandler = new lqos_ws.#Events(lqos_ws);
            lqos_ws.#Setup();
        };
        this.socketRef.onmessage = function(e) {
            new lqos_ws.#Message(JSON.parse(e.data)).process(lqos_ws.eventHandler);
        };
        this.socketRef.onclose = function(e) {
            if (!lqos_ws.socketRef || lqos_ws.socketRef.readyState == 3)
                lqos_ws.#Connect();
        };
    }
    constructor(document) {
        this.document = document;
        this.dataBuffer = new this.#DataBuffer();
        this.#Connect();
    }
    #Setup(){
        document.querySelectorAll('[data-lqos-subscribe]').forEach((subscription) => {
            this.#subscribe({subject: 'subscribe', data: {subject: subscription.getAttribute('data-lqos-subscribe'), data: subscription.getAttribute('data-lqos-data') || "", packed: false}});
        });
    }
    #subscribe(event) {
        if (event)
            new this.#Message(event).process(this.eventHandler);
    }
    unsubscribe() {
        this.socketRef.send();
    }
}

document.addEventListener("DOMContentLoaded", () => {
    var lqos_webSocket = new LqosWs(document);
});

// MultiRingBuffer provides an interface for storing multiple ring-buffers
// of performance data, with a view to them ending up on the same graph.
class MultiRingBuffer {
    constructor(capacity) {
        this.capacity = capacity;
        this.data = {};
    }

    push(id, download, upload) {
        if (!this.data.hasOwnProperty(id)) {
            this.data[id] = new RingBuffer(this.capacity);
        }
        this.data[id].push(download, upload);
    }

    plotStackedBars(target_div, rootName) {
        let graphData = [];
        for (const [k, v] of Object.entries(this.data)) {
            if (k != rootName) {
                let y = v.sortedY;
                let dn = { x: v.x_axis, y: y.down, name: k + "_DL", type: 'scatter', stackgroup: 'dn' };
                let up = { x: v.x_axis, y: y.up, name: k + "_UL", type: 'scatter', stackgroup: 'up' };
                graphData.push(dn);
                graphData.push(up);
            }
        }
        let graph = document.getElementById(target_div);
        Plotly.newPlot(
            graph,
            graphData,
            {
                margin: { l: 0, r: 0, b: 0, t: 0, pad: 4 },
                yaxis: { automargin: true },
                xaxis: { automargin: true, title: "Time since now (seconds)" },
                showlegend: false,
            },
            { responsive: true, displayModeBar: false });
    }

    plotTotalThroughput(target_div) {
        var myChart = echarts.init(target_div);
        this.data['total'].prepare();
        this.data['shaped'].prepare();
        let x = this.data['total'].x_axis;
        let option = {
            grid: {
                containLabel: true,
                left: 0,
                top: 0,
                right: 0,
                bottom: 0
            },
            xAxis: {
                data: x,
                axisLabel: {
                    interval: 30,
                    formatter: (function(value){
                        const mm_ss = new Date(value * 1000).toISOString().substring(14, 19);
                        return mm_ss;
                    })
                }
            },
            yAxis: {
                axisLabel: {
                    formatter: (function(value){
                        return bitsToSpeed(value);
                    })
                }},
            series: [
                {
                    name: 'Download',
                    showSymbol: false,
                    type: 'line',
                    itemStyle: {
                        color: 'rgb(255,160,122)'
                    },
                    data: this.data['total'].sortedY[0] - this.data['shaped'].sortedY[0],
                    stack: 'x'
                },
                {
                    name: 'Upload',
                    showSymbol: false,
                    type: 'line',
                    itemStyle: {
                        color: 'rgb(255,160,122)'
                    },
                    data: this.data['total'].sortedY[1] - this.data['shaped'].sortedY[1],
                    stack: 'x'
                },
                {
                    name: 'Shaped Download',
                    showSymbol: false,
                    type: 'line',
                    areaStyle: {
                        color: 'rgb(124,252,0)'
                    },
                    itemStyle: {
                        color: 'rgb(124,252,0)'
                    },
                    data: this.data['shaped'].sortedY[0],
                    stack: 'x'
                },
                {
                    name: 'Shaped Upload',
                    showSymbol: false,
                    type: 'line',
                    areaStyle: {
                        color: 'rgb(124,252,0)'
                    },
                    itemStyle: {
                        color: 'rgb(124,252,0)'
                    },
                    data: this.data['shaped'].sortedY[1],
                    stack: 'x'
                }
            ]
        }
        option && myChart.setOption(option);
        // let graph = document.getElementById(target_div);
        // this.data['total'].prepare();
        // this.data['shaped'].prepare();
        // let x = this.data['total'].x_axis;
        // let graphData = [
        //     {x: x, y:this.data['total'].sortedY[0], name: 'Download', type: 'scatter', marker: {color: 'rgb(255,160,122)'}},
        //     {x: x, y:this.data['total'].sortedY[1], name: 'Upload', type: 'scatter', marker: {color: 'rgb(255,160,122)'}},
        //     {x: x, y:this.data['shaped'].sortedY[0], name: 'Shaped Download', type: 'scatter', fill: 'tozeroy', marker: {color: 'rgb(124,252,0)'}},
        //     {x: x, y:this.data['shaped'].sortedY[1], name: 'Shaped Upload', type: 'scatter', fill: 'tozeroy', marker: {color: 'rgb(124,252,0)'}},
        // ];
        // if (this.plotted == null) {
        //     Plotly.newPlot(graph, graphData, { margin: { l:0,r:0,b:0,t:0,pad:4 }, yaxis: { automargin: true, title: "Traffic (bits)" }, xaxis: {automargin: true, title: "Time since now (seconds)"} }, { responsive: true });
        //     this.plotted = true;
        // } else {
        //     Plotly.redraw(graph, graphData);
        // }
    }
}

class RingBuffer {
    constructor(capacity) {
        this.capacity = capacity;
        this.head = capacity - 1;
        this.download = [];
        this.upload = [];
        this.x_axis = [];
        this.sortedY = [ [], [] ];
        for (var i = 0; i < capacity; ++i) {
            this.download.push(0.0);
            this.upload.push(0.0);
            this.x_axis.push(capacity - i);
            this.sortedY[0].push(0);
            this.sortedY[1].push(0);
        }
    }

    push(download, upload) {
        this.download[this.head] = download;
        this.upload[this.head] = 0.0 - upload;
        this.head += 1;
        this.head %= this.capacity;
    }

    prepare() {
        let counter = 0;
        for (let i=this.head; i<this.capacity; i++) {
            this.sortedY[0][counter] = this.download[i];
            this.sortedY[1][counter] = this.upload[i];
            counter++;
        }
        for (let i=0; i < this.head; i++) {
            this.sortedY[0][counter] = this.download[i];
            this.sortedY[1][counter] = this.upload[i];
            counter++;
        }
    }

    toScatterGraphData() {
        this.prepare();
        let GraphData = [
            { x: this.x_axis, y: this.sortedY[0], name: 'Download', type: 'scatter' },
            { x: this.x_axis, y: this.sortedY[1], name: 'Upload', type: 'scatter' },
        ];
        return GraphData;
    }
}

class RttHistogram {
    constructor() {
        this.entries = []
        this.x = [];
        for (let i = 0; i < 20; ++i) {
            this.entries.push(i);
            this.x.push(i * 10);
        }
    }

    clear() {
        for (let i = 0; i < 20; ++i) {
            this.entries[i] = 0;
        }
    }

    push(rtt) {
        let band = Math.floor(rtt / 10.0);
        if (band > 19) {
            band = 19;
        }
        this.entries[band] += 1;
    }

    pushBand(band, n) {
        this.entries[band] += n;
    }

    plot(target_div) {
        var myChart = echarts.init(target_div);
        let option = {
            grid: {
                containLabel: true,
                left: 10,
                top: 15,
                right: 10,
                bottom: 5
            },
            xAxis: {
                data: this.x
            },
            yAxis: {
                position: 'right',
                splitLine: {
                    show: false
                },
            },
            series: [
                {
                    type: 'bar',
                    data: this.entries
                }
            ]
        }
        myChart.setOption(option);
        // let gData = [
        //     { x: this.x, y: this.entries, type: 'bar', marker: { color: this.x, colorscale: 'RdBu' } }
        // ]
        // let graph = document.getElementById(target_div);
        // if (this.plotted == null) {
        //     Plotly.newPlot(graph, gData, { margin: { l: 40, r: 0, b: 35, t: 0 }, yaxis: { title: "# Hosts" }, xaxis: { title: 'TCP Round-Trip Time (ms)' } }, { responsive: true });
        //     this.plotted = true;
        // } else {
        //     Plotly.redraw(graph, gData);
        // }
    }
}

function metaverse_color_ramp(n) {
    if (n <= 9) {
        return "#32b08c";
    } else if (n <= 20) {
        return "#ffb94a";
    } else if (n <= 50) {
        return "#f95f53";
    } else if (n <= 70) {
        return "#bf3d5e";
    } else {
        return "#dc4e58";
    }
}

function regular_color_ramp(n) {
    if (n <= 40) {
        return "#2eb85c";
    } else if (n <= 80) {
        return "#39f";
    } else if (n <= 120) {
        return "#321fdb";
    } else if (n <= 160) {
        return "#f9b115";
    } else {
        return "#e55353";
    }
}

function color_ramp(n) {
    let colorPreference = window.localStorage.getItem("colorPreference");
    if (colorPreference == null) {
        window.localStorage.setItem("colorPreference", 0);
        colorPreference = 0;
    }
    if (colorPreference == 0) {
        return regular_color_ramp(n);
    } else {
        return metaverse_color_ramp(n);
    }
}