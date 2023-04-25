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
            //console.log("message", e.data)
            let json = JSON.parse(e.data);
            if (json.msg && json.msg == "authOk") {
                window.auth.hasCredentials = true;
                window.login = json;
                window.auth.token = json.token;
            } else if (json.msg && json.msg == "authFail") {
                window.auth.hasCredentials = false;
                window.login = null;
                window.auth.token = null;
                localStorage.removeItem("token");
                window.router.goto("login");
            }
            window.router.onMessage(json);
        };
    }

    sendToken() {
        if (window.auth.hasCredentials && window.auth.token) {
            this.ws.send(formatToken(window.auth.token));
        }
    }

    requestNodeStatus() {
        this.ws.send("{ \"msg\": \"nodeStatus\" }");
    }

    requestPacketChart() {
        this.ws.send("{ \"msg\": \"packetChart\" }");
    }

    requestThroughputChart() {
        this.ws.send("{ \"msg\": \"throughputChart\" }");
    }
}

function formatToken(token: string) {
    return "{ \"msg\": \"auth\", \"token\": \"" + token + "\" }";
}