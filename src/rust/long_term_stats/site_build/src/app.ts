import 'bootstrap/dist/css/bootstrap.css';
import 'bootstrap/dist/js/bootstrap.js';
import { SiteRouter } from './router';
import { Bus } from './bus';
import { Auth } from './auth';

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

window.setInterval(() => {
    window.router.ontick();
    window.bus.updateConnected();
}, 1000);

 function changeGraphPeriod(period: string) {
    window.graphPeriod = period;
    localStorage.setItem('graphPeriod', period);
}