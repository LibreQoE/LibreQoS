import { scaleNumber } from "../helpers";
import { mbps_to_bps } from "../site_tree/site_tree";
import { Component } from "./component";

export class CircuitInfo implements Component {
    circuitId: string;
    count: number = 0;

    constructor(siteId: string) {
        this.circuitId = siteId;
    }

    wireup(): void {
        window.bus.requestCircuitInfo(this.circuitId);
    }

    ontick(): void {
        this.count++;
        if (this.count % 10 == 0) {
            window.bus.requestCircuitInfo(this.circuitId);
        }
    }

    onmessage(event: any): void {
        if (event.msg == "circuit_info") {
            //console.log(event.data);
            let div = document.getElementById("circuitInfo") as HTMLDivElement;
            let html = "";
            html += "<table class='table table-striped'>";
            html += "<tr><td>Circuit Name:</td><td>" + event.data[0].circuit_name + "</td></tr>";
            html += "<tr><td>Min (CIR) Limits:</td><td>" + event.data[0].download_min_mbps + " / " + event.data[0].upload_min_mbps + " Mbps</td></tr>";
            html += "<tr><td>Max (Ceiling) Limits:</td><td>" + event.data[0].download_max_mbps + " / " + event.data[0].upload_max_mbps + " Mbps</td></tr>";
            html += "</table>";
            div.innerHTML = html;

            div = document.getElementById("circuitDevices") as HTMLDivElement;
            html = "";
            html += "<table class='table table-striped'>";
            for (let i=0; i<event.data.length; i++) {
                html += "<tr>";
                html += "<td>Device:</td><td>" + event.data[i].device_name + "</td>";
                html += "<td>IP:</td><td>" + event.data[i].ip_range + "/" + event.data[i].subnet + "</td>";
                html += "</tr>";
            }
            html += "</table>";
            div.innerHTML = html;
        }
    }
}