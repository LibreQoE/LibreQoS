function bytesToSize(bytes) {
    var sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    if (bytes == 0) return 'Bytes';
    var i = parseInt(Math.floor(Math.log(bytes) / Math.log(1024)));
    if (i == 0) return bytes + ' ' + sizes[i];
    return (bytes / Math.pow(1024, i)).toFixed(1) + ' ' + sizes[i];
}

function bytesToSpeed(bytes) {
    const speeds = ['bps', 'kbps', 'Mbps', 'Gbps', 'Tbps'];
    if (bytes == 0) return 'bps';
    var i = parseInt(Math.floor(Math.log(bytes) / Math.log(1024)));
    if (i == 0) return bytes + ' ' + sizes[i];
    return (bytes / Math.pow(1024, i)).toFixed(1) + ' ' + sizes[i];
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
    #Events = class {
        document;
        socketRef;
        #key_function_map = {
            update_cpu: this.#update_cpu,
            update_disk: this.#update_disk,
            update_ram: this.#update_ram,
            update_shaped_count: this.#update_shaped_count,
            update_unknown_count: this.#update_unknown_count,
        }
        constructor(lqos_ws) {
            console.log("LqosWs Event Handler Constructed! {}", lqos_ws);
            this.document = lqos_ws.document;
            this.socketRef = lqos_ws.socketRef;
        }
        dispatch(message){
            console.log("LqosWs Event: Dispatching event... {}", message.subject);
            this.document.dispatchEvent(new CustomEvent(message.subject, { detail: message }));
        }
        process(event) {
            console.log("LqosWs Event: Processing event... {}", event);
            this.#key_function_map[event.detail.subject](event.detail.data);
        }
        reload_lqos() {

        }
        subscribe(data) {
            console.log("LqosWs Event: Subscribed to... {}", data.subject);
            let event = this;
            this.document.addEventListener(data.data.subject, function(e){event.process(e)});
            this.socketRef.send(JSON.stringify(data));
        }
        #update_cpu(data) {
            console.log("LqosWs Event: Updating CPU: {}", data.subject);
            var cpu_sum = data.reduce((partialSum, a) => partialSum + a, 0);
            var cpu_avg = Math.ceil(cpu_sum/data.length);
            var spans = document.querySelectorAll("data-lqos-cpu");
            let i = 0;
            while(i<spans.length)	{
                spans[i].style.width = cpu_avg + "%";
                spans[i].innerText = cpu_avg + "%";
                i++;
            }
        }
        #update_disk(data) {
            console.log("LqosWs Event: Updating DISK: {}", data);
            var spans = document.querySelectorAll('[data-lqos-disk]');
            let i = 0;
            while(i<spans.length)	{
                spans[i].innerHTML = data;
                i++;
            }
        }
        #update_ram(data) {
            console.log("LqosWs Event: Updating RAM: {}", data);
            let consumed_ram = data[0];
            let total_ram = data[1];
            let ram_percentage = Math.ceil(consumed_ram/total_ram);
            let consumed_spans = document.querySelectorAll('[data-lqos-ram-consumed]');
            let total_spans = document.querySelectorAll('[data-lqos-ram-total]');
            let spans = document.querySelectorAll('[data-lqos-ram]');
            let i = 0;
            while(i<spans.length) {
                spans[i].style.width = ram_percentage + "%";
                spans[i].innerText = ram_percentage + "%";
                i++;
            }
            i = 0;
            while(i<consumed_spans.length) {
                consumed_spans[i].innerText = bytesToSize(consumed_ram);
                i++;
            }
            i = 0;
            while(i<total_spans.length) {
                total_spans[i].innerText = bytesToSize(total_ram);
                i++;
            }
        }
        #update_shaped_count(data) {
            console.log("LqosWs Event: Updating SHAPED COUNT: {}", data);
            let spans = document.querySelectorAll('[data-lqos-shaped-count]');
            let i = 0;
            while(i<spans.length) {
                spans[i].innerHTML = data;
                i++;
            }
        }
        #update_unknown_count(data) {
            console.log("LqosWs Event: Updating UNKNOWN COUNT: {}", data);
            let spans = document.querySelectorAll('[data-lqos-unknown-count]');
            let i = 0;
            while(i<spans.length) {
                spans[i].innerHTML = data;
                i++;
            }
        }
    }
    #Message = class Message {
        data;
        subject;
        packed = false;
        #subject_keys = {
            // Add keys here for validation
            update_cpu: "update_cpu",
            update_disk: "update_disk",
            update_ram: "update_ram",
            update_shaped_count: "update_shaped_count",
            update_unknown_count: "update_unknown_count",
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
            console.log("LqosWs Message: Processing...");
            if (this.subject && this.#subject_keys[this.subject]){
                eventHandler.dispatch(this);
            } else {
                if (this.subject == "subscribe") {
                    console.log("LqosWs Message: Subscribe request... {}", this);
                    if (this.data && this.#subject_keys[this.data.subject])
                        eventHandler.subscribe(this);
                } else if (this.subject == "unsubscribe") {
                    console.log("LqosWs Message: Unsubscribe request... {}", this);
                    if (this.data && this.#subject_keys[this.data.subject])
                        eventHandler.unsubscribe(this);
                }
            }
        }
    }
    #Connect() {
        console.log("LqosWs Connecting...");
        this.socketRef = new WebSocket('ws://' + location.host + '/ws');
        var lqos_ws = this;
        this.socketRef.onopen = function(){
            console.log("LqosWs Connected!");
            lqos_ws.eventHandler = new lqos_ws.#Events(lqos_ws);
            lqos_ws.#Setup();
        };
        this.socketRef.onmessage = function(e) {
            console.log("LqosWs Received Message... {}", e);
            new lqos_ws.#Message(JSON.parse(e.data)).process(lqos_ws.eventHandler);
        };
        this.socketRef.onclose = function(e) {
            console.log("LqosWs Closed. {}", e);
            console.log("LqosWs Reconnecting...");
            if (!lqos_ws.socketRef || lqos_ws.socketRef.readyState == 3)
                lqos_ws.#Connect();
        };
    }
    constructor(document) {
        console.log("Constructing LqosWs");
        this.document = document;
        this.#Connect();
    }
    #Setup(){
        this.subscribe({subject: 'subscribe', data: {subject: 'update_cpu', data: '', packed: false}});
        this.subscribe({subject: 'subscribe', data: {subject: 'update_disk', data: '', packed: false}});
        this.subscribe({subject: 'subscribe', data: {subject: 'update_ram', data: '', packed: false}});
        this.subscribe({subject: 'subscribe', data: {subject: 'update_shaped_count', data: '', packed: false}});
        this.subscribe({subject: 'subscribe', data: {subject: 'update_unknown_count', data: '', packed: false}});
    }
    subscribe(event) {
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