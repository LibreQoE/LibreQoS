import { Auth } from "./auth";
import { SiteRouter } from "./router";

export class Bus {
    ws: WebSocket;
    connected: boolean;

    constructor() {
        this.connected = false;
    }

    updateConnected() {
        let indicator = document.getElementById("connStatus");
        if (indicator && this.connected) {
            indicator.style.color = "green";
        } else if (indicator) {
            indicator.style.color = "red";
        }
    }

    connect() {
        const currentUrlWithoutAnchors = window.location.href.split('#')[0].replace("https://", "").replace("http://", "");
        const url = "ws://" + currentUrlWithoutAnchors + "ws";
        this.ws = new WebSocket(url);
        this.ws.onopen = () => {
            this.connected = true;
            this.sendToken();
        };
        this.ws.onclose = (e) => {
            this.connected = false;
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
        this.ws.send("{ \"msg\": \"packetChart\", \"period\": \"" + window.graphPeriod + "\" }");
    }

    requestPacketChartSingle(node_id: string, node_name: string) {
        let request = {
            msg: "packetChartSingle",
            period: window.graphPeriod,
            node_id: node_id,
            node_name: node_name,
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestThroughputChart() {
        this.ws.send("{ \"msg\": \"throughputChart\", \"period\": \"" + window.graphPeriod + "\" }");
    }

    requestThroughputChartSingle(node_id: string, node_name: string) {
        let request = {
            msg: "throughputChartSingle",
            period: window.graphPeriod,
            node_id: node_id,
            node_name: node_name,
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestThroughputChartCircuit(circuit_id: string) {
        let request = {
            msg: "throughputChartCircuit",
            period: window.graphPeriod,
            circuit_id: decodeURI(circuit_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestThroughputChartSite(site_id: string) {
        let request = {
            msg: "throughputChartSite",
            period: window.graphPeriod,
            site_id: decodeURI(site_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestRttChart() {
        this.ws.send("{ \"msg\": \"rttChart\", \"period\": \"" + window.graphPeriod + "\" }");
    }

    requestRttChartSingle(node_id: string, node_name: string) {
        let request = {
            msg: "rttChartSingle",
            period: window.graphPeriod,
            node_id: node_id,
            node_name: node_name,
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestRttChartSite(site_id: string) {
        let request = {
            msg: "rttChartSite",
            period: window.graphPeriod,
            site_id: decodeURI(site_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestNodePerfChart(node_id: string, node_name: string) {
        let request = {
            msg: "nodePerf",
            period: window.graphPeriod,
            node_id: node_id,
            node_name: node_name,
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestSiteRootHeat() {
        this.ws.send("{ \"msg\": \"siteRootHeat\", \"period\": \"" + window.graphPeriod + "\" }");
    }

    requestSiteHeat(site_id: string) {
        let request = {
            msg: "siteHeat",
            period: window.graphPeriod,
            site_id: decodeURI(site_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    sendSearch(term: string) {
        let request = {
            msg: "search",
            term: term,
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestTree(parent: string) {
        this.ws.send("{ \"msg\": \"siteTree\", \"parent\": \"" + parent + "\" }");
    }

    requestSiteInfo(site_id: string) {
        let request = {
            msg: "siteInfo",
            site_id: decodeURI(site_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestCircuitInfo(circuit_id: string) {
        let request = {
            msg: "circuitInfo",
            circuit_id: decodeURI(circuit_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }

    requestSiteParents(site_id: string) {
        let request = {
            msg: "siteParents",
            site_id: decodeURI(site_id),
        };
        let json = JSON.stringify(request);
        this.ws.send(json);
    }
}

function formatToken(token: string) {
    return "{ \"msg\": \"auth\", \"token\": \"" + token + "\" }";
}