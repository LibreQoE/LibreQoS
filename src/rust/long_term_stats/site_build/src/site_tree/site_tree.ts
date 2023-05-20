import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import mermaid from 'mermaid';
import { makeUrl, rttColor, scaleNumber, siteIcon, usageColor } from '../helpers';
import { request_node_status, request_tree } from '../../wasm/wasm_pipe';

export class SiteTreePage implements Page {
    menu: MenuPage;
    components: Component[];
    selectedNode: string;
    count: number = 0;

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
        request_node_status();
    }

    ontick(): void {
        this.menu.ontick();
        this.components.forEach(component => {
            component.ontick();
        });
        if (this.count % 10 == 0 && this.selectedNode != "") {
            fetchTree(this.selectedNode);
        }
        this.count++;
    }

    onmessage(event: any) {
        if (event.msg) {
            this.menu.onmessage(event);

            this.components.forEach(component => {
                component.onmessage(event);
            });

            if (event.msg == "NodeStatus") {
                let drop_down = document.getElementById("shaper_node_select") as HTMLSelectElement;
                if (drop_down) {
                    let items = "";
                    for (let i = 0; i < event.NodeStatus.nodes.length; i++) {
                        let isSelected = "";
                        if (i ==0 || this.selectedNode == event.NodeStatus.nodes[i].node_id) {
                            isSelected = "selected";
                            if (i == 0) {
                                this.selectedNode = event.NodeStatus.nodes[i].node_id;
                                fetchTree(this.selectedNode);
                            }
                        }
                        items += "<option " + isSelected + " value='" + event.NodeStatus.nodes[i].node_id + "'>" + event.NodeStatus.nodes[i].node_name + "</option>";
                    }
                    drop_down.innerHTML = items;
                    drop_down.onchange = () => {
                        let drop_down = document.getElementById("shaper_node_select") as HTMLSelectElement;
                        this.selectedNode = drop_down.value;
                        fetchTree(this.selectedNode);
                    };
                }
            }

            if (event.msg == "SiteTree") {
                buildTree(event.SiteTree.data);
            }
        }
    }
}

function fetchTree(parent: string) {
    request_tree(parent);
}

class TreeItem {
    index: number;
    parent: number;
    site_name: string;
    site_type: string;
    max_down: number;
    max_up: number;
    current_down: number;
    current_up: number;
    current_rtt: number;
}

export const mbps_to_bps = 1000000;

function buildTree(data: TreeItem[]) {
    data.sort((a,b) => {
        return a.site_name.localeCompare(b.site_name);
    });
    let tree = document.getElementById("site_tree") as HTMLDivElement;
    let html = "<table class='table table-striped'>";
    html += "<thead><tr><th>Site</th><th>Max</th><th>Current</th><th>Utilization</th><th>RTT (ms)</th></tr></thead>";
    html += "<tbody>";
    let def = "graph TD\n";

    for (let i=0; i<data.length; i++) {
        if (data[i].parent == 0) {
            if (data[i].site_name != "Root") {
                let up = (data[i].current_up / (data[i].max_up * mbps_to_bps)) * 100.0;
                let down = (data[i].current_down / (data[i].max_down * mbps_to_bps)) * 100.0;
                let peak = Math.max(up, down);
                let usageBg = usageColor(peak);
                let rttBg = rttColor(data[i].current_rtt / 100);
                html += "<tr>";
                let url = makeUrl(data[i].site_type, data[i].site_name);
                html += "<td>" + siteIcon(data[i].site_type) + " <a href='#" + url + "' onclick='window.router.goto(\"" + url + "\")'>" + data[i].site_name + "</a>";
                html += "</td><td>" + scaleNumber(data[i].max_down * mbps_to_bps) + " / " + scaleNumber(data[i].max_up * mbps_to_bps) + "</td>";
                html += "</td><td>" + scaleNumber(data[i].current_down) + " / " + scaleNumber(data[i].current_up) + "</td>";
                html += "<td style='background-color: " + usageBg + "'>" + up.toFixed(1) + "% / " + down.toFixed(1) + "%</td>";
                html += "<td style='background-color: " + rttBg + "'>" + (data[i].current_rtt / 100).toFixed(1) + "</td>";
                html += "</tr>";
                html += treeChildren(data, data[i].index, 1);
                def += "Root --> " + data[i].index + "[" + t(data[i].site_name) + "]\n";
                def += graphChildren(data, data[i].index, 1);
            }
        }
    }

    html += "</tbody></table>";
    tree.innerHTML = html;

    //console.log(def);
    merman(def).then(() => {});
}

async function merman(def: string) {
    let container = document.getElementById("site_map") as HTMLDivElement;
    const { svg, bindFunctions } = await mermaid.render("site_map_svg", def);
    container.innerHTML = svg;
    bindFunctions?.(container);
}

function treeChildren(data: TreeItem[], parent: number, depth: number) : string {
    let html = "";
    for (let i=0; i<data.length; i++) {
        if (data[i].parent == parent && data[i].site_name != "Root") {
            let up = (data[i].current_up / (data[i].max_up * mbps_to_bps)) * 100.0;
            let down = (data[i].current_down / (data[i].max_down * mbps_to_bps)) * 100.0;
            let peak = Math.max(up, down);
            let usageBg = usageColor(peak);
            let rttBg = rttColor(data[i].current_rtt / 100);
            html += "<tr><td>";        
            for (let j=0; j<depth; j++) {
                html += "&nbsp;&nbsp;&nbsp;&nbsp;";
            }
            let url = makeUrl(data[i].site_type, data[i].site_name);
            html += siteIcon(data[i].site_type) + " <a href='#" + url + "' onclick='window.router.goto(\"" + url + "\")'>" + data[i].site_name + "</a>";
            html += "</td><td>" + scaleNumber(data[i].max_down * mbps_to_bps) + " / " + scaleNumber(data[i].max_up * mbps_to_bps) + "</td>";
            html += "</td><td>" + scaleNumber(data[i].current_down) + " / " + scaleNumber(data[i].current_up) + "</td>";
            html += "<td style='background-color: " + usageBg + "'>" + up.toFixed(1) + "% / " + down.toFixed(1) + "%</td>";
            html += "<td style='background-color: " + rttBg + "'>" + (data[i].current_rtt / 100).toFixed(1) + "</td>";
            html += "</tr>";
            if (depth < 20) {
                html += treeChildren(data, data[i].index, depth + 1);
            }
        }
    }
    return html;
}

function t(s:string): string {
    return s.replace("(", "").replace(")", "").replace(" ", "_");
}

function graphChildren(data: TreeItem[], parent: number, depth: number) : string {
    let def = "";
    for (let i=0; i<data.length; i++) {
        if (data[i].parent == parent && data[i].site_name != "Root" && data[i].index != parent) {
            if (i < 20) def += parent + " --> " + data[i].index + "\n";
            if (depth < 20) {
                def += graphChildren(data, data[i].index, depth + 1);
            }
        }
    }
    return def;
}