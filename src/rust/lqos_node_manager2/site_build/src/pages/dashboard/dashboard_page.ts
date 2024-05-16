import html from './dashboard.html';
import {Page} from "../../page";
import {requestFlowCount, requestThroughput} from "../../requests";
import {scaleNumber} from "../../scaling";
import {ThroughputEntry, ThroughputGraph} from "../../charts/throughput_graph";
import * as echarts from 'echarts';

export class DashboardPage extends Page {
    deferredDone: boolean;
    throughputChart: ThroughputGraph | undefined;

    constructor() {
        super();
        this.deferredDone = false;
        this.throughputChart = undefined;
        this.fillContent(html);
    }

    wireup() {
        requestFlowCount();
        requestThroughput();

        // This is a fake await for after the HTML has loaded
        window.setTimeout(() => {
            this.throughputChart = new ThroughputGraph('throughputs');
            this.deferredDone = true;
        }, 1);
    }

    onmessage(event: any): void {
        switch (event.type) {
            case "FlowCount": {
                let target = document.getElementById("flowCounter");
                if (target) {
                    target.innerHTML = event.count;
                }
            } break;
            case "Throughput": {
                let target = document.getElementById("pps");
                if (target) {
                    target.innerHTML = scaleNumber(event.pps[0]) + " / " + scaleNumber(event.pps[1])
                }

                target = document.getElementById("bps");
                if (target) {
                    target.innerHTML = scaleNumber(event.bps[0]) + " / " + scaleNumber(event.bps[1])
                }

                if (this.deferredDone) {
                    this.throughputChart.onMessage(event as ThroughputEntry);
                }
            } break;
        }
    }

    ontick(): void {
        requestFlowCount();
        requestThroughput();
    }

    anchor(): string {
        return "dashboard";
    }

    replaceGraphs() {
        super.replaceGraphs();
        echarts.dispose(this.throughputChart.myChart);
        this.throughputChart = new ThroughputGraph('throughputs');
    }
}