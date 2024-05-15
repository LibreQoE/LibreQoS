import {Page} from "./page";
import {DashboardPage} from "./pages/dashboard/dashboard_page";

export class Route {
    page: String;
    target: String;

    constructor(page: String, target: String) {
        this.page = page;
        this.target = target;
    }
}

const ROUTES = [
    new Route("dashboard", "DashboardPage"),
];

export function PageFactory(route: String, params: String): Page | undefined {
    let searchFor = route.toLowerCase().trim();
    for (let i=0; i<ROUTES.length; i++) {
        if (ROUTES[i].page === searchFor) {
            return InnerFactory(ROUTES[i].target, params);
        }
    }
    return undefined;
}

function InnerFactory(target: String, params: String): Page | undefined {
    switch (target) {
        case "DashboardPage": return new DashboardPage();
        default: return undefined;
    }
}