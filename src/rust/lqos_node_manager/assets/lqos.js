function bytesToSize(bytes, fixedSize = 1) {
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    let i = parseInt(Math.floor(Math.log(bytes) / Math.log(1024)));
    if (i == 0) return bytes + ' ' + sizes[i];
    return (bytes / Math.pow(1024, i)).toFixed(fixedSize) + ' ' + sizes[i];
}

function bitsToSpeed(bits, fixedSize = 1) {
    const speeds = ['bps', 'Kbps', 'Mbps', 'Gbps', 'Tbps'];
    let upload = false;
    if (bits == 0)
        return '0bps';
    if (bits < 0)
        upload = true;
    bits = Math.abs(bits);
    let i = parseInt(Math.floor(Math.log(bits) / Math.log(1000)));
    if (upload) {
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

function scaleNumber(n) {
    if (n > 1000000000000) {
        return (n/1000000000000).toFixed(2) + 'T';
    } else if (n > 1000000000) {
        return (n/1000000000).toFixed(2) + 'G';
    } else if (n > 1000000) {
        return (n/1000000).toFixed(2) + 'M';
    } else if (n > 1000) {
        return (n/1000).toFixed(2) + 'K';
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
            document.querySelectorAll('[data-lqos-cpu]').forEach((span) => {
                span.style.width = cpu_avg + '%';
            });
        }
        #current_throughput(data) {
            if (!this.ctGraph)
                this.ctGraph = new CurrentThroughputGraph(document.querySelector('[data-lqos-subscribe="current_throughput"]'), ['unshaped_down', 'unshaped_up', 'shaped_down', 'shaped_up']);
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

            this.ctGraph.push('unshaped_down', unshaped_bits_download);
            this.ctGraph.push('unshaped_up', unshaped_bits_upload * -1);
            this.ctGraph.push('shaped_down', data['shaped_bits_per_second'][0]);
            this.ctGraph.push('shaped_up', data['shaped_bits_per_second'][1] * -1);
            
            this.ctGraph.chart.plot();
            //this.ctGraph.plotTotalThroughput(document.querySelector('[data-lqos-subscribe="current_throughput"]'));
            //this.ctGraph.plotDownload(document.querySelector('[data-lqos-subscribe="current_throughput"]'));
            //this.ctGraph.plotUpload(document.querySelector('[data-lqos-subscribe="current_throughput"]'));
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
                span.style.width = ram_percentage + '%';
            });
        }
        #rtt(data) {
            if (!this.rttGraph)
                this.rttGraph = new RttGraph(document.querySelector('[data-lqos-subscribe="rtt"]'));
            let count = 0;
            let avg = 0;
            for (let i = 0; i < data.length; i++) {
                count += data[i];
                avg += (data[i] * i) * 10;
            }
            console.log(avg);
            console.log(count);
            document.querySelector('.live-average-rtt').innerText = Math.abs(avg/count).toFixed(0);
            this.rttGraph.chart.clear();
            this.rttGraph.push(data);
        }
        #shaped_count(data) {
            document.querySelectorAll('[data-lqos-shaped-count]').forEach((span) => {
                span.innerHTML = data;
            });
        }
        #site_funnel(data) {
            let ntbody = document.createElement('tbody');
            ntbody.setAttribute('id', 'live-site-funnel');
            data.forEach((circuit) => {
                let tr = document.createElement('tr');
                let c1 = document.createElement('td');
                c1.classList.add('text-truncate');
                c1.setAttribute('width', '40%');
                c1.innerHTML = '<a href="/tree/'+ circuit[0] +'">' + circuit[1].name + '</a>';
                tr.appendChild(c1);
                let c2 = document.createElement('td');
                c2.setAttribute('width', '30%');
                let down_scale = ((circuit[1].current_throughput[0] / 100000) / circuit[1].max_throughput[0]) * 200;
                if (circuit[1].name == 'Others')
                    c2.innerHTML = '<i class="fa fa-square fa-fw pe-2" style="color: #808080;"></i>';
                else
                    c2.innerHTML = '<i class="fa fa-square fa-fw pe-2" style="color: '+color_ramp(down_scale)+';"></i>';
                c2.innerHTML += bitsToSpeed(circuit[1].current_throughput[0] * 8);
                tr.appendChild(c2);
                let c3 = document.createElement('td');
                c3.setAttribute('width', '30%');
                let up_scale = ((circuit[1].current_throughput[1] / 100000) / circuit[1].max_throughput[1]) * 200;
                if (circuit[1].name == 'Others')
                    c3.innerHTML = '<i class="fa fa-square fa-fw pe-2" style="color: #808080;"></i>';
                else
                    c3.innerHTML = '<i class="fa fa-square fa-fw pe-2" style="color: '+color_ramp(up_scale)+';"></i>';
                c3.innerHTML += bitsToSpeed(circuit[1].current_throughput[1] * 8);
                tr.appendChild(c3);
                ntbody.appendChild(tr);
            });
            let otbody = document.getElementById('live-site-funnel');
            otbody.parentNode.replaceChild(ntbody, otbody);
        }
        #top_ten_download(data) {
            let otbody = document.getElementById('live-tt');
            otbody.parentNode.replaceChild(tt_wt_tableCreator('live-tt', data), otbody);
        }
        #worst_ten_rtt(data) {
            let otbody = document.getElementById('live-wrtt');
            otbody.parentNode.replaceChild(tt_wt_tableCreator('live-wrtt', data), otbody);
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
            cpu: 'cpu',
            current_throughput: 'current_throughput',
            disk: 'disk',
            ram: 'ram',
            rtt: 'rtt',
            shaped_count: 'shaped_count',
            site_funnel: 'site_funnel',
            top_ten_download: 'top_ten_download',
            unknown_count: 'unknown_count',
            worst_ten_rtt: 'worst_ten_rtt',
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
                if (this.subject == 'subscribe') {
                    if (this.data && this.#subject_keys[this.data.subject])
                        eventHandler.subscribe(this);
                } else if (this.subject == 'unsubscribe') {
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
            this.#subscribe({subject: 'subscribe', data: {subject: subscription.getAttribute('data-lqos-subscribe'), data: subscription.getAttribute('data-lqos-data') || '', packed: false}});
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

document.addEventListener('DOMContentLoaded', () => {
    var lqos_webSocket = new LqosWs(document);
});

// MultiRingBuffer provides an interface for storing multiple ring-buffers of performance data.
class MultiRingBuffer {
    capacity;
    data;
    constructor(capacity, rings, initFn) {
        this.capacity = capacity;
        this.data = {};
        rings.forEach((ringName) => {
            this.data[ringName] = new RingBuffer(this.capacity);
        });
        initFn(this);
    }
    prepare() {
        for (const [ringName, ringBuffer] of Object.entries(this.data)) {
            let counter = 0;
            for (let i=ringBuffer.head; i<ringBuffer.capacity; i++) {
                ringBuffer.sortedY[counter] = ringBuffer.data[i];
                counter++;
            }
            for (let i=0; i < ringBuffer.head; i++) {
                ringBuffer.sortedY[counter] = ringBuffer.data[i];
                counter++;
            }
        }
    }
    push(ringName, data) {
        this.data[ringName].data.push(data);
    }
    toJSON() {
        let resp = [];
        for (const [ringName, ringBuffer] of Object.entries(this.data)) {
            resp.push({name: ringName, data: JSON.stringify(ringBuffer)});
        }
        return resp
    }
}

class RingBuffer {
    capacity;
    data;
    head;
    sortedY;
    constructor(capacity) {
        this.capacity = capacity;
        this.head = this.capacity - 1;
        this.data = [];
        this.sortedY = [];
        for (var i = 0; i < this.capacity; ++i) {
            this.data.push(0.0);
            this.sortedY.push(0);
        }
    }
    push(data) {
        this.data[this.head] = data;
        this.head += 1;
        this.head %= this.capacity;
    }
    toJSON() {
        return {
            sortedY: this.sortedY,
            data: this.data
        }
    }
}

class CurrentThroughputGraph {
    #axisTick = [];
    #capacity = 300;
    #rings = ['default'];
    static series() {
        return [
            { name: 'Unshaped Download', showSymbol: false, type: 'line', itemStyle: { color: 'rgb(255,160,122)' }, data: [], stack: 'x' },
            { name: 'Unshaped Upload', showSymbol: false, type: 'line', itemStyle: { color: 'rgb(255,160,122)' }, data: [], stack: 'x' },
            { name: 'Shaped Download', showSymbol: false, type: 'line', areaStyle: { color: 'rgb(124,252,0)' }, itemStyle: { color: 'rgb(124,252,0)' }, data: [], stack: 'x' },
            { name: 'Shaped Upload', showSymbol: false, type: 'line', areaStyle: { color: 'rgb(124,252,0)' }, itemStyle: { color: 'rgb(124,252,0)' }, data: [], stack: 'x' }
        ]
    }
    #options = {
        xAxis: { data: this.#axisTick, axisLabel: { interval: 30, formatter: (function(value) { return ssToMmss(value); }) } },
        yAxis: { axisLabel: { formatter: (function(value) { return bitsToSpeed(value); }) } },
        series: CurrentThroughputGraph.series()
    }
    constructor(targetDom, rings) {
        this.rings = rings;
        for (var i = 0; i < this.#capacity; ++i)
            this.#axisTick.push(this.#capacity - i);
        this.chart = new LineGraph(targetDom, this.#capacity, function(input) {
            input.multiRingBuffer = new MultiRingBuffer(input.capacity, rings, function(mrb) {
                mrb.toJSON = function() {
                    let resp = [];
                    for (const [index, [key, value]] of Object.entries(Object.entries(mrb.data))) {
                        resp.push(Object.assign(CurrentThroughputGraph.series()[index], {data: value.sortedY}));
                    }
                    return resp;
                }
            });
        });
        this.chart.prepare(this.#options);
        this.chart.plot(); 
    }
    push(ringName, data) {
        this.chart.push(ringName, data);
    }
}

class LineGraph {
    capacity;
    chart;
    multiRingBuffer;
    options = {
        grid: { containLabel: true, left: 5, top: 0, right: 0, bottom: 0 }
    }
    constructor(targetDom, capacity, initFn) {
        this.chart = echarts.init(targetDom);
        this.capacity = capacity;
        initFn(this);
    }
    plot() {
        this.multiRingBuffer.prepare();
        this.options = Object.assign(this.options, { series: JSON.parse(JSON.stringify(this.multiRingBuffer)) });
        this.chart.setOption(this.options);
    }
    prepare(options) {
        this.options = Object.assign(options, this.options);
    }
    push(ringName, data) {
        this.multiRingBuffer.data[ringName].push(data);
    }
}

class RttGraph {
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
    constructor(target_dom) {
        for (var i = 0; i < this.#capacity; ++i) {
            this.#axisTick.push(i * 10);
        }
        this.chart = new Histogram(target_dom, this.#capacity, function(input) {
            input.ringBuffer = new RingBuffer(input.capacity);
        });
        this.chart.prepare(this.#options);
        this.chart.plot();
    }
    push(data) {
        this.chart.push(data);
    }
}

class Histogram {
    #capacity;
    chart;
    ringBuffer;
    #options = {
        xAxis: {},
        yAxis: {}
    }
    constructor(targetDom, capacity, initFn) {
        this.chart = echarts.init(targetDom);
        this.#capacity = capacity;
        initFn(this);
        for (let i = 0; i < this.#capacity; ++i)
            this.ringBuffer.data.push(i);
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

function tt_wt_tableCreator(id, data) {
    const ntbody = document.createElement('tbody');
    ntbody.setAttribute('id', id);
    data.forEach((circuit) => {
        let tr = document.createElement('tr');
        let c1 = document.createElement('td');
        c1.classList.add('text-truncate');
        if (circuit.circuit_id)
            c1.innerHTML = '<a class="redact" href="/circuit/'+ circuit.circuit_id +'">' + circuit.ip_address + '</a>';
        else
            c1.innerText = circuit.ip_address;
        tr.appendChild(c1);
        let c2 = document.createElement('td');
        c2.innerHTML = bitsToSpeed(circuit.bits_per_second[0]);
        tr.appendChild(c2);
        let c3 = document.createElement('td');
        c3.innerHTML = bitsToSpeed(circuit.bits_per_second[1]);
        tr.appendChild(c3);
        let c4 = document.createElement('td');
        c4.innerHTML = '<i class="fa fa-square fa-fw pe-2" style="color: '+color_ramp(circuit.median_tcp_rtt)+';"></i>';
        c4.innerHTML += circuit.median_tcp_rtt.toFixed(2);
        tr.appendChild(c4);
        let c5 = document.createElement('td');
        if (circuit.tc_handle != 0) {
            c5.innerHTML = '<i class="fa fa-circle-check text-success fa-fw pe-2"></i>';
            c5.innerHTML += circuit.plan[0] + '/' + circuit.plan[1];
        } else {
            c5.innerHTML = '<i class="fa fa-circle-xmark text-danger fa-fw pe-2"></i>';
            c5.innerHTML += 'N/A';
        }
        tr.appendChild(c5);
        ntbody.appendChild(tr);
    });
    return ntbody;
}