import html from './template.html';

export class LoginPage {
    constructor() {
        document.body.innerHTML = html;
        let button = document.getElementById('btnLogin');
        if (button)
            button.onclick = () => { alert('blah') };
    }
}