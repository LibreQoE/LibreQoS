import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { NodeList } from '../components/node_list';

export class ShaperNodesPage implements Page {
    menu: MenuPage;
    components: Component[]

    constructor() {
        this.menu = new MenuPage("nodesDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new NodeList(),
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