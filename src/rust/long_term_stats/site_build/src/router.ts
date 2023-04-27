import { Auth } from './auth';
import { DashboardPage } from './dashboard/dashboard';
import { LoginPage } from './login/login';
import { Page } from './page';
import { ShaperNodePage } from './shapernode/shapernode';
import { ShaperNodesPage } from './shapernodes/shapernodes';

export class SiteRouter {
    curentPage: Page | undefined;
    currentAnchor: string;

    constructor() {
        this.curentPage = undefined;
        this.currentAnchor = "";
    }

    initialRoute() {
        if (window.auth.hasCredentials) {
            let container = document.getElementById('main');
            if (container) {
                container.innerHTML = "<i class=\"fa-solid fa-spinner fa-spin\"></i>";
            }
            window.setTimeout(() => {         
                let target = window.location.hash;
                if (target == "" || target == "#") {
                    target = "dashboard";
                }
                this.goto(target);
            }, 1000);
        } else {
            this.goto("login");
        }
    }

    ontick() {
        if (this.curentPage) {
            this.curentPage.ontick();
        }
    }

    // Handle actual navigation between pages
    goto(page: string) {
        page = page.replace('#', '');
        //console.log("Navigate to " + page)
        let split = page.split(':');
        switch (split[0].toLowerCase()) {
            case "login": {
                this.currentAnchor = "login";
                this.curentPage = new LoginPage();
                break;
            }
            case "dashboard": {
                this.currentAnchor = "dashboard";
                this.curentPage = new DashboardPage();
                break;
            }
            case "shapernodes": {
                this.currentAnchor = "shapernodes";
                this.curentPage = new ShaperNodesPage();
                break;
            }
            case "shapernode": {
                this.currentAnchor = "shapernode:" + split[1] + ":" + split[2];
                this.curentPage = new ShaperNodePage(split[1], split[2]);
                break;
            }
            default: {
                alert("I don't know how to go to: " + split[0].toLowerCase());
                this.goto("dashboard");
                return;
            }
        }
        window.location.hash = this.currentAnchor;
        this.curentPage.wireup();
    }

    onMessage(event: any) {
        if (this.curentPage) {
            this.curentPage.onmessage(event);
        }
    }
}