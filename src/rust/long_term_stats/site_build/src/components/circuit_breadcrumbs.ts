import { request_circuit_parents } from "../../wasm/wasm_pipe";
import { makeUrl } from "../helpers";
import { Component } from "./component";

export class CircuitBreadcrumbs implements Component {
    circuitId: string;

    constructor(siteId: string) {
        this.circuitId = siteId;
    }

    wireup(): void {
        request_circuit_parents(this.circuitId);
    }

    ontick(): void {
    }

    onmessage(event: any): void {
        if (event.msg == "SiteParents") {
            //console.log(event.data);
            let div = document.getElementById("siteName") as HTMLDivElement;
            let html = "";
            let crumbs = event.SiteParents.data.reverse();
            for (let i = 0; i < crumbs.length; i++) {
                let url = makeUrl(crumbs[i][0], crumbs[i][1]);
                html += "<a href='#" + url + "' onclick='window.router.goto(\"" + url + "\")'>" + crumbs[i][1] + "</a> | ";
            }
            html = html.substring(0, html.length - 3);
            div.innerHTML = html;
        }
    }
}