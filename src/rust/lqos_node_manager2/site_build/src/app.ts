import { SiteRouter } from './router'
import {Bus} from "./bus";
import {InitDayNightMode} from "./darkmode";
import {registerThemes} from "./charts/echarts_themes";

declare global {
    interface Window {
        router: SiteRouter;
        bus: Bus;
    }
}

InitDayNightMode();
registerThemes();
window.bus = new Bus();
window.router = new SiteRouter();
window.router.initialRoute();

// WebSocket Connection Ticker
window.setInterval(() => {
    window.bus.updateConnected();
    if (!window.bus.connected) {
        window.bus = new Bus();
    }
}, 500);

// Ticker goes here
window.setInterval(() => {
    window.router.onTick();
}, 1000);