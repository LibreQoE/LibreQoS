import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { PacketsChartSingle } from '../components/packets_single';
import { RttHisto } from '../components/rtt_histo';
import { ThroughputChartSingle } from '../components/throughput_single';
import { RttChartSingle } from '../components/rtt_graph_single';
import { NodeCpuChart } from '../components/node_cpu';
import { NodeRamChart } from '../components/node_ram';

export class ShaperNodePage implements Page {
    menu: MenuPage;
    components: Component[]
    node_id: string;
    node_name: string;

    constructor(node_id: string, node_name: string) {
        this.node_id = node_id;
        this.node_name = node_name;
        this.menu = new MenuPage("nodesDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new PacketsChartSingle(this.node_id, this.node_name),
            new ThroughputChartSingle(this.node_id, this.node_name),
            new RttChartSingle(this.node_id, this.node_name),
            new RttHisto(),
            new NodeCpuChart(this.node_id, this.node_name),
            new NodeRamChart(this.node_id, this.node_name),
        ];
        let name = document.getElementById('nodeName');
        if (name) {
            name.innerText = "Shaper Node: " + this.node_name;
        }
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