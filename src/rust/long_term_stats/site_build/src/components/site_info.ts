import { scaleNumber } from "../helpers";
import { mbps_to_bps } from "../site_tree/site_tree";
import { Component } from "./component";
import { request_site_info } from "../../wasm/wasm_pipe";

export class SiteInfo implements Component {
    siteId: string;
    count: number = 0;

    constructor(siteId: string) {
        this.siteId = siteId;
    }

    wireup(): void {
        request_site_info(decodeURI(this.siteId));
    }

    ontick(): void {
        this.count++;
        if (this.count % 10 == 0) {
            request_site_info(decodeURI(this.siteId));
        }
    }

    onmessage(event: any): void {
        if (event.msg == "SiteInfo") {
            //console.log(event.data);
            let div = document.getElementById("siteInfo") as HTMLDivElement;
            let html = "";
            html += "<table class='table table-striped'>";
            html += "<tr><td>Max:</td><td>" + scaleNumber(event.SiteInfo.data.max_down * mbps_to_bps) + " / " + scaleNumber(event.SiteInfo.data.max_up * mbps_to_bps) + "</td></tr>";
            html += "<tr><td>Current:</td><td>" + scaleNumber(event.SiteInfo.data.current_down) + " / " + scaleNumber(event.SiteInfo.data.current_up) + "</td></tr>";
            html += "<tr><td>Current RTT:</td><td>" + event.SiteInfo.data.current_rtt / 100.0 + " ms</td></tr>";
            html += "</table>";
            div.innerHTML = html;
        }
    }
}