import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { NodeStatus } from '../components/node_status';
import { PacketsChart } from '../components/packets';
import { ThroughputChart } from '../components/throughput';
import { RttChart } from '../components/rtt_graph';
import { RttHisto } from '../components/rtt_histo';
import { RootHeat } from '../components/root_heat';
import { SiteStackChart } from '../components/site_stack';
import { RootBreadcrumbs } from '../components/root_breadcrumbs';

export class DashboardPage implements Page {
    menu: MenuPage;
    components: Component[]

    constructor() {
        this.menu = new MenuPage("menuDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new RootBreadcrumbs(),
            new PacketsChart(),
            new ThroughputChart(),
            new RttChart(),
            new RttHisto(),
            new RootHeat(),
            new SiteStackChart("root"),
        ];
        window.toggleThroughput = toggleThroughput;
        window.toggleLatency = toggleLatency;
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

function toggleThroughput(mode: string) {
    let elements = [
        document.getElementById("bitsholder"),
        document.getElementById("packetsholder"),
        document.getElementById("sitestackholder"),
    ];

    // Clear all
    elements.forEach(element => {
        if (element) {
            element.style.height = "0px";
            element.style.overflow = "none";
        }
    });

    var element = 0;
    switch (mode) {
        case "tp": { var element = 0 } break;
        case "pk": { var element = 1 } break;
        case "st": { var element = 2 } break;
    }

    let e = elements[element];
    if (e) {
        e.style.height = "250px";
        e.style.overflow = "auto";
    }
}

function toggleLatency(mode: string) {
    let elements = [
        document.getElementById("rttHistoHolder"),
        document.getElementById("rttChartHolder"),
    ];

    // Clear all
    elements.forEach(element => {
        if (element) {
            element.style.height = "0px";
            element.style.overflow = "none";
        }
    });

    var element = 0;
    switch (mode) {
        case "histo": { var element = 0 } break;
        case "line": { var element = 1 } break;
    }

    let e = elements[element];
    if (e) {
        e.style.height = "250px";
        e.style.overflow = "auto";
    }
}