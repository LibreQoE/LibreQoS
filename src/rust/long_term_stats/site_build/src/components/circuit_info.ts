import { scaleNumber } from "../helpers";
import { mbps_to_bps } from "../site_tree/site_tree";
import { Component } from "./component";
import { request_circuit_info } from "../../wasm/wasm_pipe";

export class CircuitInfo implements Component {
    circuitId: string;
    count: number = 0;

    constructor(siteId: string) {
        this.circuitId = decodeURI(siteId);
    }

    wireup(): void {
        request_circuit_info(this.circuitId);
    }

    ontick(): void {
        this.count++;
        if (this.count % 10 == 0) {
            request_circuit_info(this.circuitId);
        }
    }

    onmessage(event: any): void {
        if (event.msg == "CircuitInfo") {
            //console.log(event.CircuitInfo.data);
            let div = document.getElementById("circuitInfo") as HTMLDivElement;
            let html = "";
            html += "<table class='table table-striped'>";
            html += "<tr><td>Circuit Name:</td><td>" + event.CircuitInfo.data[0].circuit_name + "</td></tr>";
            html += "<tr><td>Min (CIR) Limits:</td><td>" + event.CircuitInfo.data[0].download_min_mbps + " / " + event.CircuitInfo.data[0].upload_min_mbps + " Mbps</td></tr>";
            html += "<tr><td>Max (Ceiling) Limits:</td><td>" + event.CircuitInfo.data[0].download_max_mbps + " / " + event.CircuitInfo.data[0].upload_max_mbps + " Mbps</td></tr>";
            html += "</table>";
            div.innerHTML = html;

            div = document.getElementById("circuitDevices") as HTMLDivElement;
            html = "";
            html += "<table class='table table-striped'>";
            for (let i=0; i<event.CircuitInfo.data.length; i++) {
                html += "<tr>";
                html += "<td>Device:</td><td>" + event.CircuitInfo.data[i].device_name + "</td>";
                html += "<td>IP:</td><td>" + event.CircuitInfo.data[i].ip_range + "/" + event.CircuitInfo.data[i].subnet + "</td>";
                html += "</tr>";
            }
            html += "</table>";
            div.innerHTML = html;
        }
    }
}