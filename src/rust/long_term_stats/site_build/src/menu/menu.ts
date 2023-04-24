import html from './template.html';
import { Page } from '../page'

export class MenuPage implements Page {
    activePanel: string;

    constructor(activeElement: string) {
        let container = document.getElementById('main');
        if (container) {
            container.innerHTML = html;

            let activePanel = document.getElementById(activeElement);
            if (activePanel) {
                activePanel.classList.add('active');
            }

            let username = document.getElementById('menuUser');
            if (username) {
                if (window.login) {
                    username.textContent = window.login.name;
                } else {
                    username.textContent = "Unknown";
                }
            }
        }
    }

    wireup() {
    }    

    onmessage(event: any) {
        if (event.msg) {
        }
    }

    ontick(): void {
        // Do nothing
    }
}