export class Auth {
    hasCredentials: boolean;
    token: string | undefined;

    constructor() {
        let token = localStorage.getItem("token");
        if (token) {
            this.hasCredentials = true;
            this.token = token;
        } else {
            this.hasCredentials = false;
            this.token = undefined;
        }
    }
}