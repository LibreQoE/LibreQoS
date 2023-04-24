import { Auth } from './auth';
import { LoginPage } from './login/login';
import { Page } from './page';

export class SiteRouter {
    curentPage: Page | undefined;

    constructor() {
        this.curentPage = undefined;
    }

    initialRoute() {
        if (window.auth.hasCredentials) {
            this.goto("dashboard");
        } else {
            this.goto("login");
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