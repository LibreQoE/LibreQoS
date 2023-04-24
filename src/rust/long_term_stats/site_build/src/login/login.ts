import html from './template.html';
import { Page } from '../page'
import { getValueFromForm } from '../helpers';

export class LoginPage implements Page {
    constructor() {
        let container = document.getElementById('main');
        if (container) {
            container.innerHTML = html;        
        }
    }

    wireup() {
        // Connect the button
        let button = document.getElementById('btnLogin');
        if (button) {
            button.onclick = this.onLogin;
        }

        let stored_license = localStorage.getItem('license');
        if (stored_license) {
            let input = document.getElementById('license') as HTMLInputElement;
            if (input) {
                input.value = stored_license;
            }
        }

        // Set focus
        let focusTarget = "license";
        if (stored_license) {
            focusTarget = "username";
        }
        let input = document.getElementById(focusTarget);
        if (input) {
            input.focus();
        }
    }    

    onLogin() {
        let license = getValueFromForm('license');
        let username = getValueFromForm('username');
        let password = getValueFromForm('password');
    
        if (license == "") {
            alert("Please enter a license key");
            return;
        }
        if (username == "") {
            alert("Please enter a username");
            return;
        }
        if (password == "") {
            alert("Please enter a password");
            return;
        }

        localStorage.setItem('license', license);

        let btn = document.getElementById('btnLogin');
        if (btn) {
            btn.innerHTML = "<i class=\"fa-solid fa-spinner fa-spin\"></i>";
        }

        let data = {
            msg: "login",
            license: license,
            username: username,
            password: password,
        };
        let json: string = JSON.stringify(data);
        window.bus.ws.send(json);
    }

    onmessage(event: any) {
        if (event.msg) {
            if (event.msg == "loginOk") {
                // TODO: Store the credentials globally
                window.router.goto("dashboard");
            } else if (event.msg = "loginFailed") {
                alert("Login failed");
                let btn = document.getElementById('btnLogin');
                if (btn) {
                    btn.textContent = "Login";
                }
            }
        }
    }
}