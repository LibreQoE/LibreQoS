// Setup any WS feeds for this page
import { ws_proto } from '../../pubsub/ws.js';
let ws = null;

export function subscribeWS(channels, handler) {
    if (channels.length === 0) {
        return;
    }
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
