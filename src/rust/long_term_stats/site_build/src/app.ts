import 'bootstrap/dist/css/bootstrap.css';
import 'bootstrap/dist/js/bootstrap.js';
import { SiteRouter } from './router';
import { Bus, onAuthFail, onAuthOk, onMessage } from './bus';
import { Auth } from './auth';
import init from '../wasm/wasm_pipe.js';

await init();
console.log("WASM loaded");

declare global {
    interface Window {
        router: SiteRouter;
        bus: Bus;
        auth: Auth;
        login: any;
        graphPeriod: string;
        changeGraphPeriod: any;
    }
}
(window as any).onAuthFail = onAuthFail;
(window as any).onAuthOk = onAuthOk;
(window as any).onMessage = onMessage;

window.auth = new Auth;
window.bus = new Bus();
window.router = new SiteRouter();
window.bus.connect();
window.router.initialRoute();
let graphPeriod = localStorage.getItem('graphPeriod');
if (!graphPeriod) {
    graphPeriod = "5m";
    localStorage.setItem('graphPeriod', graphPeriod);
}
window.graphPeriod = graphPeriod;
window.changeGraphPeriod = (period: string) => changeGraphPeriod(period);

// 10 Second interval for refreshing the page
window.setInterval(() => {
    window.bus.updateConnected();    
    window.router.ontick();
    let btn = document.getElementById("graphPeriodBtn") as HTMLButtonElement;
    btn.innerText = window.graphPeriod;
}, 10000);

// Faster interval for tracking the WSS connection
window.setInterval(() => {
    window.bus.updateConnected();
    window.bus.sendQueue();
}, 500);

function changeGraphPeriod(period: string) {
    window.graphPeriod = period;
    localStorage.setItem('graphPeriod', period);
    let btn = document.getElementById("graphPeriodBtn") as HTMLButtonElement;
    btn.innerText = period;
}
