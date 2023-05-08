import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';

export class SiteTreePage implements Page {
    menu: MenuPage;
    components: Component[];
    selectedNode: string;

    constructor() {
        this.menu = new MenuPage("sitetreeDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
        ];
        this.selectedNode = "";
    }

    wireup() {
        this.components.forEach(component => {
            component.wireup();
        });
        window.bus.requestNodeStatus();
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

            if (event.msg == "nodeStatus") {
                let drop_down = document.getElementById("shaper_node_select") as HTMLSelectElement;
                if (drop_down) {
                    let items = "";
                    for (let i = 0; i < event.nodes.length; i++) {
                        let isSelected = "";
                        if (i ==0 || this.selectedNode == event.nodes[i].node_id) {
                            isSelected = "selected";
                            if (i == 0) {
                                this.selectedNode = event.nodes[i].node_id;
                                fetchTree(this.selectedNode);
                            }
                        }
                        items += "<option " + isSelected + " value='" + event.nodes[i].node_id + "'>" + event.nodes[i].node_name + "</option>";
                    }
                    drop_down.innerHTML = items;
                    drop_down.onchange = () => {
                        let drop_down = document.getElementById("shaper_node_select") as HTMLSelectElement;
                        this.selectedNode = drop_down.value;
                        fetchTree(this.selectedNode);
                    };
                }
            }

            if (event.msg == "site_tree") {
                buildTree(event.data);
            }
        }
    }
}

function fetchTree(parent: string) {
    window.bus.requestTree(parent);
}

class TreeItem {
    index: number;
    parent: number;
    site_name: string;
    site_type: string;
}

function buildTree(data: TreeItem[]) {
    console.log(data);
    let tree = document.getElementById("site_tree") as HTMLDivElement;
    let html = "";

    for (let i=0; i<data.length; i++) {
        if (data[i].parent == 0) {
            html += "(" + data[i].site_type + ") " + data[i].site_name + "<br />";
            html += treeChildren(data, data[i].index, 1);
        }
    }

    tree.innerHTML = html;
}

function treeChildren(data: TreeItem[], parent: number, depth: number) : string {
    let html = "";
    for (let i=0; i<data.length; i++) {
        if (data[i].parent == parent && data[i].site_name != "Root") {
            for (let j=0; j<depth; j++) {
                html += "&nbsp;&nbsp;&nbsp;&nbsp;";
            }
            html += "(" + data[i].site_type + ") " + data[i].site_name + "<br />";
            if (depth < 20) {
                treeChildren(data, data[i].index, depth + 1);
            }
        }
    }
    return html;
}