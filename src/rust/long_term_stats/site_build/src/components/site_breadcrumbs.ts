import { request_site_parents } from "../../wasm/wasm_pipe";
import { makeUrl } from "../helpers";
import { Component } from "./component";

export class SiteBreadcrumbs implements Component {
    siteId: string;

    constructor(siteId: string) {
        this.siteId = siteId;
    }

    wireup(): void {
        request_site_parents(this.siteId);
    }

    ontick(): void {
    }

    onmessage(event: any): void {
        if (event.msg == "SiteParents") {
            //console.log(event.data);
            let div = document.getElementById("siteName") as HTMLDivElement;
            let html = "";
            let crumbs = event.SiteParents.data.reverse();
            for (let i = 0; i < crumbs.length-1; i++) {
                let url = makeUrl(crumbs[i][0], crumbs[i][1]);
                html += "<a href='#" + url + "' onclick='window.router.goto(\"" + url + "\")'>" + crumbs[i][1] + "</a> | ";
            }
            html += crumbs[crumbs.length-1][1] + " | ";
            html += "<select id='siteChildren'></select>";
            div.innerHTML = html;
        } else if (event.msg == "SiteChildren") {
            //console.log(event.data);
            let html = "<option value=''>-- Children --</option>";
            for (let i=0; i<event.SiteChildren.data.length; i++) {
                html += "<option value='" + makeUrl(event.SiteChildren.data[i][0], event.SiteChildren.data[i][1]) + "'>" + event.SiteChildren.data[i][2] + "</option>";
            }
            let select = document.getElementById("siteChildren") as HTMLSelectElement;
            select.innerHTML = html;
            select.onchange = () => {
                let select = document.getElementById("siteChildren") as HTMLSelectElement;
                window.router.goto(select.value);
            };
        }
    }
}