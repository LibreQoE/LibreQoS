let ws: WebSocket;
let tried: boolean = false;

export class Bus {
    connected: boolean = false;

    constructor() {
        if (!tried) {
            const currentUrlWithoutAnchors = window.location.href.split('#')[0].replace("https://", "").replace("http://", "");
            // Figure out where we're going
            let url = "ws://" + currentUrlWithoutAnchors + "ws";
            if (window.location.href.startsWith("https://")) {
                url = "wss:/" + currentUrlWithoutAnchors + "ws";
            }

            // Connect
            console.log("Attempting to connect websocket to: " + url);
            ws = new WebSocket(url);
            ws.onclose = onClose;
            ws.onopen = onOpen;
            ws.onmessage = onMessage;
            ws.onerror = onError;
            tried = true;
        }
    }

    send(msg: any) {
        let json = JSON.stringify(msg);
        this.sendString(json);
    }

    sendString(msg: string) {
        ws.send(msg);
    }

    updateConnected() {
        let indicator = document.getElementById("connStatus");
        if (indicator && this.connected) {
            indicator.style.color = "green";
        } else if (indicator) {
            indicator.style.color = "red";
        }
    }
}

function onOpen(event: any) {
    console.log("WS Connected");
    window.bus.connected = true;
    ws.send(JSON.stringify({
        "type" : "hello"
    }));
}

function onClose(event: any) {
    window.bus.connected = false;
    tried = false;
}

function onError(event: any) {
    window.bus.connected = false;
    tried = false;
}

function onMessage(event: any) {
    let message = JSON.parse(event.data);
    window.router.onMessage(message);
}