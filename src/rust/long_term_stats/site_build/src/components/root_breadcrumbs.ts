import { request_root_parents } from "../../wasm/wasm_pipe";
import { makeUrl } from "../helpers";
import { Component } from "./component";

export class RootBreadcrumbs implements Component {
    constructor() {
    }

    wireup(): void {
        let div = document.getElementById("siteName") as HTMLDivElement;
        div.innerHTML = "Root | <select id='siteChildren'></select>";
        request_root_parents();
    }

    ontick(): void {
    }

    onmessage(event: any): void {
        if (event.msg == "SiteChildren") {
            //console.log(event.data);
            let html = "<option value=''>-- Children --</option>";
            for (let i=0; i<event.SiteChildren.data.length; i++) {
                if (event.SiteChildren.data[i][1] != "Root") {
                    html += "<option value='" + makeUrl(event.SiteChildren.data[i][0], event.SiteChildren.data[i][1]) + "'>" + event.SiteChildren.data[i][2] + "</option>";
                }
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