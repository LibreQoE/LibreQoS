import 'bootstrap/dist/css/bootstrap.css';
import { SiteRouter } from './router';
import { Bus } from './bus';
import { Auth } from './auth';

declare global {
    interface Window {
        router: SiteRouter;
        bus: Bus;
        auth: Auth;
    }
}

window.auth = new Auth;
window.bus = new Bus();
window.router = new SiteRouter();
window.bus.connect();
window.router.initialRoute();
