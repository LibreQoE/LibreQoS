// Setup any WS feeds for this page
let ws = null;

export function ws_proto() {
    if (window.location.protocl === 'https') {
        return "wss://";
    } else {
        return "ws://";
    }
}

export function subscribeWS(channels, handler) {
    if (ws) {
        ws.close();
    }

    ws = new WebSocket(ws_proto() + window.location.host + '/websocket/ws');
    ws.onopen = () => {
        for (let i=0; i<channels.length; i++) {
            ws.send("{ \"channel\" : \"" + channels[i] + "\"}");
        }
    }
    ws.onclose = () => {
        ws = null;
    }
    ws.onerror = (error) => {
        ws = null
    }
    ws.onmessage = function (event) {
        let msg = JSON.parse(event.data);
        handler(msg);
    };
}

export function resetWS() {
    ws = null;
}