import html from './dashboard.html';
import {Page} from "../../page";
import {requestFlowCount} from "../../requests";

export class DashboardPage extends Page {
    constructor() {
        super();
        this.fillContent(html);
    }

    wireup() {
        requestFlowCount();
    }

    onmessage(event: any): void {
        switch (event.type) {
            case "FlowCount": {
                let target = document.getElementById("flowCounter");
                if (target) {
                    target.innerHTML = event.count;
                }
            } break;
        }
    }

    ontick(): void {
        requestFlowCount();
    }

    anchor(): string {
        return "dashboard";
    }
}