import { Auth } from './auth';
import { DashboardPage } from './dashboard/dashboard';
import { LoginPage } from './login/login';
import { Page } from './page';

export class SiteRouter {
    curentPage: Page | undefined;

    constructor() {
        this.curentPage = undefined;
    }

    initialRoute() {
        if (window.auth.hasCredentials) {
            let container = document.getElementById('main');
            if (container) {
                container.innerHTML = "<i class=\"fa-solid fa-spinner fa-spin\"></i>";
            }
            window.setTimeout(() => {                
                this.goto("dashboard");
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
        console.log("Navigate to " + page)
        switch (page) {
            case "login": {
                this.curentPage = new LoginPage();
                break;
            }
            case "dashboard": {
                this.curentPage = new DashboardPage();
                break;
            }
            default: {
                alert("I don't know how to go to: " + page);
                return;
            }
        }
        this.curentPage.wireup();
    }

    onMessage(event: any) {
        if (this.curentPage) {
            this.curentPage.onmessage(event);
        }
    }
}