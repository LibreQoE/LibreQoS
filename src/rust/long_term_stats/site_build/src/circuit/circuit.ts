import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';

export class CircuitPage implements Page {
    menu: MenuPage;
    components: Component[];
    siteId: string;

    constructor(siteId: string) {
        this.siteId = siteId;
        this.menu = new MenuPage("sitetreeDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
        ];
    }

    wireup() {
        this.components.forEach(component => {
            component.wireup();
        });
    }

    ontick(): void {
        this.menu.ontick();
        this.components.forEach(component => {
            component.ontick();
        });
    }

    onmessage(event: any) {
        if (event.msg) {
            this.menu.onmessage(event);

            this.components.forEach(component => {
                component.onmessage(event);
            });
        }
    }
}
