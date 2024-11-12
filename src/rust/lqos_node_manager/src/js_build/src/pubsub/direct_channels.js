export class DirectChannel {
    constructor(subObject, handler) {
        this.ws = null;
        this.handler = handler;
        this.ws = new WebSocket('ws://' + window.location.host + '/websocket/private_ws');
        this.ws.onopen = () => {
            this.ws.send(JSON.stringify(subObject));
        }
        this.ws.onclose = () => {
            this.ws = null;
        }
        this.ws.onerror = (error) => {
            this.ws = null
        }
        this.ws.onmessage = function (event) {
            let msg = JSON.parse(event.data);
            handler(msg);
        };
    }
}