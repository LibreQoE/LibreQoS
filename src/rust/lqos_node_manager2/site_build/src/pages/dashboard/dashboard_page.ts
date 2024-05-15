import html from './dashboard.html';
import {Page} from "../../page";

export class DashboardPage extends Page {
    constructor() {
        super();
        this.fillContent(html);
    }

    wireup() {
        requestFlowCount();
    }

    onmessage(event: any): void {
        if (event.type === "FlowCount") {
            let target = document.getElementById("flowCounter");
            if (target) {
                target.innerHTML = event.count;
            }
        }
    }

    ontick(): void {
        console.log("Dash Tick");
        requestFlowCount();
    }

    anchor(): string {
        return "dashboard";
    }
}

function requestFlowCount() {
    window.bus.send({
        "type" : "flowcount"
    })
}