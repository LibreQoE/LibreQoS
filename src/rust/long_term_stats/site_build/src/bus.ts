import { Auth } from "./auth";
import { SiteRouter } from "./router";

export class Bus {
    ws: WebSocket;

    constructor() {
    }

    connect() {
        this.ws = new WebSocket("ws://192.168.100.10:9127/ws");
        this.ws.onopen = () => {
            let indicator = document.getElementById("connStatus");
            if (indicator) {
                indicator.style.color = "green";
            }
            this.sendToken();
        };
        this.ws.onclose = (e) => {
            let indicator = document.getElementById("connStatus");
            if (indicator) {
                indicator.style.color = "red";
            }
            console.log("close", e) 
        };
        this.ws.onerror = (e) => { console.log("error", e) };
        this.ws.onmessage = (e) => { 
            console.log("message", e.data) 
            window.router.onMessage(e.data);
        };
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