import html from './template.html';
import { Page } from '../page'
import { getValueFromForm } from '../helpers';

export class LoginPage implements Page {
    constructor() {
        document.body.innerHTML = html;        
    }

    wireup() {
        // Connect the button
        let button = document.getElementById('btnLogin');
        if (button) {
            button.onclick = this.onLogin;
        }

        // Set focus
        let input = document.getElementById('license');
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

        let data = {
            msg: "login",
            license: license,
            username: username,
            password: password,
        };
        let json: string = JSON.stringify(data);
        window.bus.ws.send(json);
    }
}