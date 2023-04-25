import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';

export class DashboardPage implements Page {
    menu: MenuPage;

    constructor() {
        this.menu = new MenuPage("menuDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
    }

    wireup() {
        window.bus.requestNodeStatus();
    }    

    ontick(): void {
        this.menu.ontick();
        window.bus.requestNodeStatus();
    }

    onmessage(event: any) {
        if (event.msg) {
            this.menu.onmessage(event);

            if (event.msg == "nodeStatus") {
                let status = document.getElementById("nodeStatus");
                let html = "";
                if (status) {
                    for (let i = 0; i < event.nodes.length; i++) {
                        let node = event.nodes[i];
                        let color = "danger";
                        if (node.last_seen > 86400) {
                            color = "secondary";
                        } 
                        if (node.last_seen < 60) {
                            color = "warning";
                        } 
                        if (node.last_seen < 20) {
                            color = "success";
                        }
                        html += "<span class='badge rounded-pill text-bg-" + color + "'><i class='fa-solid fa-server'></i> " + node.node_id + "</span> ";
                    }
                    status.innerHTML = html;
                }                
            }
        }
    }
}