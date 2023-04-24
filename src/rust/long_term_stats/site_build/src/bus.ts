import { Auth } from "./auth";
import { SiteRouter } from "./router";

export class Bus {
    ws: WebSocket;

    constructor() {
    }

    connect() {
        this.ws = new WebSocket("ws://192.168.100.10:9127/ws");
        this.ws.onopen = () => { 
            this.sendToken();
        };
        this.ws.onclose = (e) => { console.log("close", e) };
        this.ws.onerror = (e) => { console.log("error", e) };
        this.ws.onmessage = (e) => { console.log("message", e.data) };
    }

    sendToken() {
        if (window.auth.hasCredentials && window.auth.token) {
            this.ws.send(formatToken(window.auth.token));
        }
    }
}

function formatToken(token: string) {
    return "{ msg: \"Auth\", token: \"" + token + "\" }";
}