import { LoginPage } from './login/login';
import { Page } from './page';

export class SiteRouter {
    hasCredentials: boolean;
    curentPage: Page | undefined;

    constructor() {
        this.curentPage = undefined;
        let token = localStorage.getItem("token");
        if (token) {
            this.hasCredentials = true;
        } else {
            this.hasCredentials = false;
        }
    }

    initialRoute() {
        if (this.hasCredentials) {
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
}