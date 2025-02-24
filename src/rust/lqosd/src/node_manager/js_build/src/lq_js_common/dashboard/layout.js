export class DashboardLayout {
    constructor(cookieName, defaultLayout) {
        this.cookieName = cookieName;
        let template = localStorage.getItem(cookieName);
        if (template !== null) {
            this.dashlets = JSON.parse(template);
        } else {
            this.dashlets = defaultLayout;
        }
    }

    save(dashletIdentities) {
        this.dashlets = dashletIdentities;
        let template = JSON.stringify(dashletIdentities);
        localStorage.setItem(this.cookieName, template);
    }
}