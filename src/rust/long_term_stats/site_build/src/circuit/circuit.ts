import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { CircuitInfo } from '../components/circuit_info';
import { ThroughputCircuitChart } from '../components/throughput_circuit';
import { RttChartCircuit } from '../components/rtt_circuit';
import { request_ext_device_info } from "../../wasm/wasm_pipe";

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
        request_ext_device_info(this.circuitId);
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

            if (event.msg == "DeviceExt") {
                console.log(event.DeviceExt.data);
                let div = document.getElementById("ext") as HTMLDivElement;
                let html = "";

                for (let i=0; i<event.DeviceExt.data.length; i++) {
                    let d = event.DeviceExt.data[i];
                    html += "<div class='row'>";
                    html += "<div class='col-6'>";
                    html += "<h4>" + d.name + "</h4>";
                    html += "<strong>Status</strong>: " + d.status + "<br>";
                    html += "<strong>Model</strong>: " + d.model + "<br>";
                    html += "<strong>Mode</strong>: " + d.mode + "<br>";
                    html += "<strong>Firmware</strong>: " + d.firmware + "<br>";
                    html += "</div>";
                    html += "<div class='col-6'>";
                    html += "<p>Signal/noise graph</p>";
                    html += "</div>";
                    html += "</div>";
                }

                div.outerHTML = html;
            }
        }
    }
}
