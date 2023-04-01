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
            cpu: this.#cpu,
            disk: this.#disk,
            ram: this.#ram,
            shaped_count: this.#shaped_count,
            unknown_count: this.#unknown_count,
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
            this.#key_function_map[event.detail.subject](JSON.parse(event.detail.data));
        }
        reload_lqos() {

        }
        subscribe(data) {
            console.log("LqosWs Event: Subscribed to... {}", data.subject);
            let event = this;
            this.document.addEventListener(data.data.subject, function(e){event.process(e)});
            this.socketRef.send(JSON.stringify(data));
        }
        #cpu(data) {
            console.log("LqosWs Event: Updating CPU: {}", data);
            var cpu_sum = data.reduce((partialSum, a) => partialSum + a, 0);
            var cpu_avg = Math.ceil(cpu_sum/data.length);
            document.querySelectorAll("data-lqos-cpu").forEach((span) => {
                span.style.width = cpu_avg + "%";
                span.innerText = cpu_avg + "%";
            });
        }
        #disk(data) {
            console.log("LqosWs Event: Updating DISK: {}", data);
            document.querySelectorAll('[data-lqos-disk]').forEach((span) => {
                span.innerHTML = data;
            });
        }
        #ram(data) {
            console.log("LqosWs Event: Updating RAM: {}", data);
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
                span.innerText = ram_percentage + "%";
            });
        }
        #shaped_count(data) {
            console.log("LqosWs Event: Updating SHAPED COUNT: {}", data);
            document.querySelectorAll('[data-lqos-shaped-count]').forEach((span) => {
                span.innerHTML = data;
            });
        }
        #unknown_count(data) {
            console.log("LqosWs Event: Updating UNKNOWN COUNT: {}", data);
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
            disk: "disk",
            ram: "ram",
            shaped_count: "shaped_count",
            unknown_count: "unknown_count",
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
        document.querySelectorAll('[data-lqos-subscribe]').forEach((subscription) => {
            this.subscribe({subject: 'subscribe', data: {subject: subscription.getAttribute('data-lqos-subscribe'), data: subscription.getAttribute('data-lqos-data') || "", packed: false}});
        });
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