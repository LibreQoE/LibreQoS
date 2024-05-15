import {Page} from "./page";
import {PageFactory} from "./routes";

export class SiteRouter {
    curentPage: Page | undefined;
    currentAnchor: string;

    constructor() {
        this.curentPage = undefined;
        this.currentAnchor = "";
    }

    initialRoute() {
        // TODO: Check for credentials
        window.setTimeout(() => {
            let target = window.location.hash;
            if (target === "" || target === "#") {
                target = "dashboard";
            }
            this.goto(target);
        }, 1000);
    }

    ontick() {
        if (this.curentPage) {
            this.curentPage.ontick();
        }
    }

    goto(page: String) {
        page = page.replace('#', '');
        let split = page.split(':');
        let params = "";
        if (split.length > 1) {
            params = split[1];
        }
        let maybe_page = PageFactory(split[0], params);
        if (maybe_page === undefined) {
            alert("I don't know how to go to: " + split[0]);
            this.goto("dashboard");
            return;
        }
        this.curentPage = maybe_page;
        this.currentAnchor = this.curentPage.anchor();
        window.location.hash = this.currentAnchor;
        this.curentPage.wireup();
    }

    onMessage(event: any) {
        if (this.curentPage) {
            this.curentPage.onmessage(event);
        }
    }

    onTick() {
        if (this.curentPage) {
            this.curentPage.ontick();
        }
    }
}