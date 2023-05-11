import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { CircuitInfo } from '../components/circuit_info';
import { ThroughputCircuitChart } from '../components/throughput_circuit';
import { RttChartCircuit } from '../components/rtt_circuit';

export class CircuitPage implements Page {
    menu: MenuPage;
    components: Component[];
    circuitId: string;

    constructor(circuitId: string) {
        this.circuitId = circuitId;
        this.menu = new MenuPage("sitetreeDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new CircuitInfo(this.circuitId),
            new ThroughputCircuitChart(this.circuitId),
            new RttChartCircuit(this.circuitId),
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
