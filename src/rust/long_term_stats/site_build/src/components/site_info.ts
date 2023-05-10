import { scaleNumber } from "../helpers";
import { mbps_to_bps } from "../site_tree/site_tree";
import { Component } from "./component";

export class SiteInfo implements Component {
    siteId: string;
    count: number = 0;

    constructor(siteId: string) {
        this.siteId = siteId;
    }

    wireup(): void {
        window.bus.requestSiteInfo(this.siteId);
    }

    ontick(): void {
        this.count++;
        if (this.count % 10 == 0) {
            window.bus.requestSiteInfo(this.siteId);
        }
    }

    onmessage(event: any): void {
        if (event.msg == "site_info") {
            //console.log(event.data);
            (document.getElementById("siteName") as HTMLElement).innerText = event.data.site_name;
            let div = document.getElementById("siteInfo") as HTMLDivElement;
            let html = "";
            html += "<table class='table table-striped'>";
            html += "<tr><td>Max:</td><td>" + scaleNumber(event.data.max_down * mbps_to_bps) + " / " + scaleNumber(event.data.max_up * mbps_to_bps) + "</td></tr>";
            html += "<tr><td>Current:</td><td>" + scaleNumber(event.data.current_down) + " / " + scaleNumber(event.data.current_up) + "</td></tr>";
            html += "<tr><td>Current RTT:</td><td>" + event.data.current_rtt / 100.0 + " ms</td></tr>";
            html += "</table>";
            div.innerHTML = html;
        }
    }
}