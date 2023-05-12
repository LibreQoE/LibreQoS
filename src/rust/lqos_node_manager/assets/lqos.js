function average(array, fixedSize = 1) {
    if (array.length == 0) return 0;
    return (array.reduce((t, c) => t + c) / array.length).toFixed(fixedSize);
}

function bytesToSize(bytes, fixedSize = 1) {
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    let i = parseInt(Math.floor(Math.log(bytes) / Math.log(1024)));
    if (i == 0) return bytes + ' ' + sizes[i];
    return (bytes / Math.pow(1024, i)).toFixed(fixedSize) + ' ' + sizes[i];
}

function bitsToSpeed(bits, fixedSize = 1, label = false) {
    const speeds = ['bps', 'Kbps', 'Mbps', 'Gbps', 'Tbps'];
    let upload = false;
    if (bits == 0)
        return '0bps';
    if (bits < 0)
        upload = true;
    bits = Math.abs(bits);
    let i = parseInt(Math.floor(Math.log(bits) / Math.log(1000)));
    if (!label && upload) {
        if (i == 0) return bits + ' ' + speeds[i];
        return -Math.abs((bits / Math.pow(1000, i))).toFixed(fixedSize) + speeds[i];
    } else {
        if (i == 0) return bits + ' ' + speeds[i];
        return (bits / Math.pow(1000, i)).toFixed(fixedSize) + speeds[i];
    }
}

function ssToMmss(seconds) {
    const mm_ss = new Date(seconds * 1000).toISOString().substring(14, 19);
    return mm_ss;
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

function toggleRedact(src) {
	var event = document.createEvent('Event');
	if (document.body.classList.contains('dark-theme')) {
		document.body.classList.remove('dark-theme');
	} else {
		document.body.classList.add('dark-theme');
	}
	document.body.dispatchEvent(event);
}

function scaleNumber(n, fixedSize = 2, label = false) {
    const counts = ['', 'K', 'M', 'G', 'T'];
    let upload = false;
    if (n == 0)
        return '0';
    if (n < 0)
        upload = true;
    n = Math.abs(n);
    let i = parseInt(Math.floor(Math.log(n) / Math.log(1000)));
    if (!label && upload) {
        if (i == 0) return n + ' ' + counts[i];
        return -Math.abs((n / Math.pow(1000, i))).toFixed(fixedSize) + counts[i];
    } else {
        if (i == 0) return n + ' ' + counts[i];
        return (n / Math.pow(1000, i)).toFixed(fixedSize) + counts[i];
    }
}

class LqosEvent {
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
        pcap: this.#pcap,
    }
    process(event) {
        // The entire event is here, not just the message. This allows for some server side control.
        // the event.detail.instructor attribute allows us to handle the data in any manner desirable
        // i.e. update/remove data, send chart capacity limits, whatever can be wrapped in json
        console.log("triggering event {}", event)
        if (this.#key_function_map[event.detail.message.subject])
            this.#key_function_map[event.detail.message.subject](JSON.parse(event.detail));
    }
    #cpu(event) {
        let data = event.message.content;
        var cpu_sum = data.reduce((partialSum, a) => partialSum + a, 0);
        var cpu_avg = Math.ceil(cpu_sum/data.length);
        document.querySelectorAll('[data-lqos-cpu]').forEach((span) => {
            span.style.width = cpu_avg + '%';
        });
    }
    #current_throughput(event) {
        let data = event.message.content;
        if (!this.ctGraph)
            this.ctGraph = new CurrentThroughputGraph(document.querySelector('[data-lqos-subscribe="current_throughput"]'));
        let unshaped_bits_download = data['bits_per_second'][0] - data['shaped_bits_per_second'][0];
        let unshaped_bits_upload = data['bits_per_second'][1] - data['shaped_bits_per_second'][1];
        document.querySelector('.live-download-packets').innerText = scaleNumber(data['packets_per_second'][0]);
        document.querySelector('.live-upload-packets').innerText = scaleNumber(data['packets_per_second'][1]);
        document.querySelector('.live-shaped-download').innerText = bitsToSpeed(data['shaped_bits_per_second'][0], 2);
        document.querySelector('.live-shaped-upload').innerText = bitsToSpeed(data['shaped_bits_per_second'][1], 2);
        document.querySelector('.live-unshaped-download').innerText = bitsToSpeed(unshaped_bits_download, 2);
        document.querySelector('.live-unshaped-upload').innerText = bitsToSpeed(unshaped_bits_upload, 2);
        document.querySelector('.live-download').innerText = bitsToSpeed(data['bits_per_second'][0], 2);
        document.querySelector('.live-upload').innerText = bitsToSpeed(data['bits_per_second'][1], 2);
        // If we can split up the sent data to be each their own key: value we can push the array 
        // in one go rather than 6 seperate
        // this.ctGraph.push(data);
        this.ctGraph.push('unshaped_down', unshaped_bits_download);
        this.ctGraph.push('unshaped_up', unshaped_bits_upload * -1);
        this.ctGraph.push('shaped_down', data['shaped_bits_per_second'][0]);
        this.ctGraph.push('shaped_up', data['shaped_bits_per_second'][1] * -1);
        this.ctGraph.push('packets_down', data['packets_per_second'][0]);
        this.ctGraph.push('packets_up', data['packets_per_second'][1] * -1);
        this.ctGraph.chart.plot();
    }
    #disk(event) {
        let data = event.message.content;
        document.querySelectorAll('[data-lqos-disk]').forEach((span) => {
            span.innerHTML = data;
        });
    }
    #pcap() {

    }
    #ram(event) {
        let data = event.message.content;
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
            span.style.width = ram_percentage + '%';
        });
    }
    #rtt(event) {
        let data = event.message.content;
        if (!this.rttGraph)
            this.rttGraph = new RttGraph(document.querySelector('[data-subscribe-subject="rtt"]'));
        let count = 0;
        let avg = 0;
        for (let i = 0; i < data.length; i++) {
            count += data[i];
            avg += (data[i] * i) * 10;
        }
        document.querySelector('.live-average-rtt').innerText = Math.abs(avg/count).toFixed(0);
        this.rttGraph.chart.clear();
        this.rttGraph.push(data);
    }
    #shaped_count(event) {
        let data = event.message.content;
        document.querySelectorAll('[data-lqos-shaped-count]').forEach((span) => {
            span.innerHTML = data;
        });
    }
    #site_funnel(event) {
        let data = event.message.content;
        let ntbody = document.createElement('tbody');
        ntbody.setAttribute('id', 'live-site-funnel');
        data.forEach((circuit) => {
            let tr = document.createElement('tr');
            let c1 = document.createElement('td');
            c1.classList.add('text-truncate');
            c1.setAttribute('width', '40%');
            c1.innerHTML = '<i class="fa fa-square fa-fw pe-2" data-bs-toggle="tooltip" data-bs-title="' + average(circuit[1].rtts) + '" style="color: ' + color_ramp(average(circuit[1].rtts)) + ';"></i><a href="/tree/'+ circuit[0] +'">' + redact(circuit[1].name) + '</a>';
            tr.appendChild(c1);
            let c2 = document.createElement('td');
            c2.classList.add('text-end');
            c2.setAttribute('width', '30%');
            let down_scale = ((circuit[1].current_throughput[0] / 100000) / circuit[1].max_throughput[0]) * 200;
            c2.innerHTML = bitsToSpeed(circuit[1].current_throughput[0] * 8);
            if (circuit[1].name == 'Others')
                c2.innerHTML += '<i class="fa fa-down-long fa-fw ps-2" style="color: #808080;"></i>';
            else
                c2.innerHTML += '<i class="fa fa-down-long fa-fw ps-2" style="color: '+color_ramp(down_scale)+';"></i>';
            tr.appendChild(c2);
            let c3 = document.createElement('td');
            c3.classList.add('text-end');
            c3.setAttribute('width', '30%');
            let up_scale = ((circuit[1].current_throughput[1] / 100000) / circuit[1].max_throughput[1]) * 200;
            c3.innerHTML = bitsToSpeed(circuit[1].current_throughput[1] * 8);
            if (circuit[1].name == 'Others')
                c3.innerHTML += '<i class="fa fa-up-long fa-fw ps-2" style="color: #808080;"></i>';
            else
                c3.innerHTML += '<i class="fa fa-up-long fa-fw ps-2" style="color: '+color_ramp(up_scale)+';"></i>';
            tr.appendChild(c3);
            ntbody.appendChild(tr);
        });
        let otbody = document.getElementById('live-site-funnel');
        otbody.parentNode.replaceChild(ntbody, otbody);
    }
    #top_ten_download(event) {
        let data = event.message.content;
        let otbody = document.getElementById('live-tt');
        otbody.parentNode.replaceChild(tt_wt_tableCreator('live-tt', data), otbody);
    }
    #unknown_count(event) {
        let data = event.message.content;
        document.querySelectorAll('[data-lqos-unknown-count]').forEach((span) => {
            span.innerHTML = data;
        });
    }
    #worst_ten_rtt(event) {
        let data = event.message.content;
        let otbody = document.getElementById('live-wrtt');
        otbody.parentNode.replaceChild(tt_wt_tableCreator('live-wrtt', data), otbody);
    }
}

class LqosWs {
    constructor(document) {
        this.document = document;
        this.#Connect();
    }
    document;
    socketRef;
    #Connect() {
        this.socketRef = new WebSocket('wss://' + location.host + '/ws');
        let lqos_ws = this;
        this.socketRef.onopen = function(){
            // Dispatch an event stating we connected and are ready for requests
            document.dispatchEvent(new Event('LqosWsConnected'));
        };
        this.socketRef.onmessage = function(e) {
            let raw_event = JSON.parse(e.data);
            new lqos_ws.#Event(raw_event, true, lqos_ws.document);
        };
        this.socketRef.onclose = function(e) {
            // Dispatch an event stating we disconnected and are not ready for requests
            document.dispatchEvent(new Event('LqosWsDisconnected'));
            if (!lqos_ws.socketRef || lqos_ws.socketRef.readyState == 3)
                lqos_ws.#Connect();
        };
    }
    #Event = class {
        constructor(event, a = false, document = false) {
            this.message = new this.#Message(event.message);
            this.task = event.task;
            if ((typeof a == "boolean" && a) && document)
                this.#DispatchEvent(document);
        }
        message;
        task;
        #DispatchEvent(document) {
            document.dispatchEvent(new CustomEvent(this.message.subject, { detail: this }));
        }
        #Message = class {
            constructor(message) {
                this.content = message.content;
                this.packed = message.packed;
                this.subject = message.subject;
                (message.packed) && this.#decode();
            }
            content;
            packed;
            subject;
            #decode() {
                this.content = msgpack.decode(new Uint8Array(this.content));
            }
            toJSON() {
                return {
                    content: this.content,
                    packed: this.packed,
                    subject: this.subject,
                }
            }
        }
        toJSON() {
            return {
                message: this.message.toJSON(),
                task: this.task,
            }
        }
    }
    #Send(data) {
        this.socketRef.send(data);
    }
    process_event(raw_event) {
        let event = new this.#Event(raw_event);
        if (event.subject == 'subscribe') {
            console.log("subscribing event: {}", event);
            this.document.addEventListener(event.message.subject, function(e){LqosEvent.process(e)});
        }
        else if (event.subject == 'unsubscribe') {
            console.log("unsubscribing event: {}", event);
            this.document.removeEventListener(event.message.subject, function(e){LqosEvent.process(e)});
        }
        this.#Send(JSON.stringify(event));
    }
}
var lqos_ws;
document.addEventListener('DOMContentLoaded', () => {
    lqos_ws = new LqosWs(document);
});

document.addEventListener('LqosWsConnected', () => {
    document.querySelectorAll('[data-subscribe]').forEach((subscription) => {
        lqos_ws.process_event({
            message: {
                content: subscription.getAttribute('data-subscribe-content') || '',
                packed: false,
                subject: subscription.getAttribute('data-subscribe-subject'),
            },
            task: 'subscribe',
        });
    });
});

// TODO: Create a store of manually triggered subscriptions to resubscribe to upon loss of connection

document.addEventListener('LqosWsDisconnected', () => {
    // TODO: Handle cleanup of all event listeners as not to create duplicates when reconnecting.
    // Maybe show a toast that the connection has been lost as an indicator, or the dreadful 
    // spinning logo saying waiting... oh the irony!
});

// Handles data storage and graph creation and management
class Graphing {
    // MultiRingBuffer provides an interface for storing multiple ring-buffers.
    MultiRingBuffer = class {
        constructor(capacity, rings, initFn, reversed = false) {
            this.capacity = capacity;
            this.data = {};
            rings.forEach((ringName) => {
                this.data[ringName] = new LqosGraphing.RingBuffer(this.capacity, reversed);
            });
            initFn(this);
        }
        capacity;
        data;
        push(ringName, data = false) {
            if (typeof ringName == "object") {
                Object.keys(ringName).forEach((k) => {
                    this.data[k].data.push(ringName[k])
                });
            } else {
                this.data[ringName].data.push(data);
            }
        }
        toJSON() {
            let resp = [];
            for (const [ringName, ringBuffer] of Object.entries(this.data)) {
                resp.push({name: ringName, data: ringBuffer.data});
            }
            return resp
        }
    }
    RingBuffer = class {
        constructor(capacity, reversed = false) {
            this.capacity = capacity;
            this.#reversed = reversed;
            this.data = Array.from({ length: this.capacity }, ()=> (0));
        }
        capacity;
        data;
        head = 0;
        #reversed;
        push(data) {
            if (this.#reversed) {
                this.data.unshift(data);
                this.data.pop();
            } else {
                this.data.push(data);
                this.data.shift();
            }
        }
    }
    LineGraph = class {
        capacity;
        chart;
        multiRingBuffer;
        options = {
            grid: { containLabel: true, left: 5, top: 0, right: 5, bottom: 0 },
        } 
        constructor(targetDom, capacity, initFn) {
            this.chart = echarts.init(targetDom);
            this.capacity = capacity;
            initFn(this);
        }
        plot() {
            this.options = Object.assign({}, this.options, { series: JSON.parse(JSON.stringify(this.multiRingBuffer)) });
            this.chart.setOption(this.options);
        }
        prepare(options) {
            this.options = Object.assign({}, options, this.options);
        }
        push(ringName, data) {
            this.multiRingBuffer.push(ringName, data);
        }
    }
    Histogram = class {
        constructor(targetDom, capacity, initFn) {
            this.chart = echarts.init(targetDom);
            this.#capacity = capacity;
            initFn(this);
            for (let i = 0; i < this.#capacity; ++i)
                this.ringBuffer.data.push(i);
        }
        #capacity;
        chart;
        ringBuffer;
        #options = {
            xAxis: {},
            yAxis: {}
        }
        clear() {
            for (let i = 0; i < this.#capacity; ++i)
                this.ringBuffer.data[i] = 0;
        }
        push(data) {
            for (let i = 0; i < data.length; i++)
                this.ringBuffer.data[i] = data[i];
            this.plot();
        }
        prepare(options) {
            this.#options = Object.assign(this.#options, options);
        }
        plot() {
            this.#options = Object.assign(this.#options, { series: [ { type: 'bar', data: this.ringBuffer.data } ] })
            this.chart.setOption(this.#options);
        }
    }
}

class CurrentThroughputGraph {
    constructor(targetDom) {
        for (var i = 0; i < this.#capacity; ++i)
            this.#axisTick.push(this.#capacity - i);
        let rings = this.rings;
        this.chart = new Graphing.LineGraph(targetDom, this.#capacity, function(lineGraph) {
            lineGraph.multiRingBuffer = new Graphing.MultiRingBuffer(lineGraph.capacity, rings, function(mrb) {
                mrb.toJSON = function() {
                    let resp = [];
                    for (const [index, [key, value]] of Object.entries(Object.entries(mrb.data))) {
                        resp.push(Object.assign(CurrentThroughputGraph.series()[index], {data: value.data}));
                    }
                    return resp;
                }
            });
        }, this.#reversed);
        this.chart.prepare(this.#options);
        this.chart.plot(); 
    }
    #axisTick = [];
    #capacity = 300;
    #reversed = true;
    rings = ['shaped_down', 'shaped_up', 'unshaped_down', 'unshaped_up', 'packets_down', 'packets_up'];
    static series() {
        return [
            { name: 'Shaped Download', showSymbol: false, type: 'line', areaStyle: { color: rampColors.regular[0] }, itemStyle: { color: rampColors.regular[0] }, data: [], stack: 'x', lineStyle: { normal: { width: 1 } }, yAxisIndex: 0 },
            { name: 'Shaped Upload', showSymbol: false, type: 'line', areaStyle: { color: rampColors.regular[0] }, itemStyle: { color: rampColors.regular[0] }, data: [], stack: 'x', lineStyle: { normal: { width: 1 } }, yAxisIndex: 0 },
            { name: 'Unshaped Download', showSymbol: false, type: 'line', areaStyle: { color: rampColors.regular[3] }, itemStyle: { color: rampColors.regular[3] }, data: [], stack: 'x', lineStyle: { normal: { width: 1 } }, yAxisIndex: 0 },
            { name: 'Unshaped Upload', showSymbol: false, type: 'line', areaStyle: { color: rampColors.regular[3] }, itemStyle: { color: rampColors.regular[3] }, data: [], stack: 'x', lineStyle: { normal: { width: 1 } }, yAxisIndex: 0 },
            { name: 'Packets Download', showSymbol: false, type: 'line', itemStyle: { color: rampColors.regular[5] }, data: [], lineStyle: { normal: { width: 1 } }, yAxisIndex: 1 },
            { name: 'Packets Upload', showSymbol: false, type: 'line', itemStyle: { color: rampColors.regular[5] }, data: [], lineStyle: { normal: { width: 1 } }, yAxisIndex: 1 }
        ]
    }
    #options = {
        xAxis: { data: this.#axisTick, axisLabel: { interval: 20, formatter: (function(value) { return ssToMmss(value); }) } },
        yAxis: [{ axisLabel: { formatter: (function(value) { return bitsToSpeed(value, 1, true); }) } }, { axisLabel: { formatter: (function(value) { return scaleNumber(value, 1, true); }) } }],
        tooltip: { trigger: 'axis', axisPointer: { type: 'cross' }, valueFormatter: (value) => bitsToSpeed(value, 1, true), },
        series: this.series()
    }
    push(ringName, data) {
        this.chart.push(ringName, data);
    }
}

class RttGraph {
    constructor(target_dom) {
        for (var i = 0; i < this.#capacity; ++i)
            this.#axisTick.push(i * 10);
        this.chart = new Histogram(target_dom, this.#capacity, function(input) {
            input.ringBuffer = new RingBuffer(input.capacity);
        });
        this.chart.prepare(this.#options);
        this.chart.plot();
    }
    #axisTick = [];
    #capacity = 20;
    #count = 0;
    #options = {
        grid: { containLabel: true, left: 10, top: 15, right: 10, bottom: 5 },
        xAxis: { data: this.#axisTick },
        yAxis: { position: 'right', splitLine: { show: false }, },
        series: [ ],
        animationDuration: 300,
        animationDurationUpdate: 700,
        animationEasing: 'linear',
        animationEasingUpdate: 'linear'
    }
    push(data) {
        this.chart.push(data);
    }
}

const rampColors = {
    meta: ['#32b08c', '#ffb94a', '#f95f53', '#bf3d5e', '#dc4e58'],
    regular: ['#2eb85c', '#39f', '#321fdb', '#ffc107', '#fd7e14', '#e55353'],
};

function metaverse_color_ramp(n) {
    if (n <= 9) {
        return rampColors.meta[0];
    } else if (n <= 20) {
        return rampColors.meta[1];
    } else if (n <= 50) {
        return rampColors.meta[2];
    } else if (n <= 70) {
        return rampColors.meta[3];
    } else {
        return rampColors.meta[4];
    }
}

function regular_color_ramp(n) {
    if (n <= 33) {
        return rampColors.regular[0];
    } else if (n <= 66) {
        return rampColors.regular[1];
    } else if (n <= 99) {
        return rampColors.regular[2];
    } else if (n <= 132) {
        return rampColors.regular[3];
    } else if (n <= 165) {
        return rampColors.regular[4];
    } else {
        return rampColors.regular[5];
    }
}

function color_ramp(n) {
    let colorPreference = window.localStorage.getItem('colorPreference');
    if (colorPreference == null) {
        window.localStorage.setItem('colorPreference', 0);
        colorPreference = 0;
    }
    if (colorPreference == 0) {
        return regular_color_ramp(n);
    } else {
        return metaverse_color_ramp(n);
    }
}

function genRandomString(string) {
    const cap_alpha_chars = 'ABCDEFGHIJKLMNOPQRSTUVQXYZ';
    const alpha_chars = 'abcdefghijklmnopqrstuvwxyz';
    const numer_chars = '0123456789';
    let result = '';
    for (let char of string) {
        if (/^[A-Z]*$/.test(char)) {
            result += cap_alpha_chars.charAt(Math.floor(Math.random() * cap_alpha_chars.length));
        } else if (/^[a-z]*$/.test(char)) {
            result += alpha_chars.charAt(Math.floor(Math.random() * alpha_chars.length));
        } else if (/^[0-9]*$/.test(char)) {
            result += numer_chars.charAt(Math.floor(Math.random() * numer_chars.length));
        } else {
            result += char;
        }
    }
    return result;
 }

function isRedacted() {
    let redact = localStorage.getItem("redact");
    if (redact == null) {
        localStorage.setItem("redact", false);
        redact = false;
    }
    if (redact == "false") {
        redact = false;
    } else if (redact == "true") {
        redact = true;
    }
    return redact;
}

function redact(string) {
    if (!isRedacted()) return string;
    return genRandomString(string);
}

function tt_wt_tableCreator(id, data) {
    const ntbody = document.createElement('tbody');
    ntbody.setAttribute('id', id);
    data.forEach((circuit) => {
        let tr = document.createElement('tr');
        let c1 = document.createElement('td');
        c1.classList.add('text-truncate');
        c1.classList.add('text-nowrap');
        c1.setAttribute('width', '40%');
        c1.innerHTML = '<i class="fa fa-square fa-fw pe-2" data-bs-toggle="tooltip" data-bs-title="' + circuit.median_tcp_rtt.toFixed(2) + '" style="color: ' + color_ramp(circuit.median_tcp_rtt) + ';"></i>';
        if (circuit.circuit_id)
            c1.innerHTML += '<a class="redact text-nowrap" href="/circuit/'+ circuit.circuit_id +'">' + redact(circuit.ip_address) + '</a>';
        else
            c1.innerHTML += redact(circuit.ip_address);
        tr.appendChild(c1);
        let c2 = document.createElement('td');
        c2.setAttribute('width', '20%');
        c2.classList.add('text-end');
        c2.innerHTML = bitsToSpeed(circuit.bits_per_second[0]);
        tr.appendChild(c2);
        let c3 = document.createElement('td');
        c3.setAttribute('width', '20%');
        c3.classList.add('text-end');
        c3.innerHTML = bitsToSpeed(circuit.bits_per_second[1]);
        tr.appendChild(c3);
        let c4 = document.createElement('td');
        c4.setAttribute('width', '20%');
        c4.classList.add('text-end');
        if (circuit.tc_handle != 0) {
            c4.innerHTML = '<i class="fa fa-circle-check text-success fa-fw pe-2"></i>';
            c4.innerHTML += circuit.plan[0] + '/' + circuit.plan[1];
        } else {
            c4.innerHTML = '<i class="fa fa-circle-xmark text-danger fa-fw pe-2"></i>';
            c4.innerHTML += 'N/A';
        }
        tr.appendChild(c4);
        ntbody.appendChild(tr);
    });
    return ntbody;
}