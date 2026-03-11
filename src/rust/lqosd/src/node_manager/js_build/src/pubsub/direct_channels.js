import { get_ws_client } from "./ws";

export class DirectChannel {
    constructor(subObject, handler) {
        this.client = get_ws_client();
        this.handler = handler;
        this.event_name = Object.keys(subObject)[0];
        this.bound_handler = (msg) => {
            handler(msg);
        };
        this.dispose_handler = this.client.on(this.event_name, this.bound_handler);
        this.client.send({ Private: subObject });
    }

    close() {
        if (!this.client || !this.bound_handler) {
            return;
        }
        if (this.dispose_handler) {
            this.dispose_handler();
            this.dispose_handler = null;
        } else {
            this.client.off(this.event_name, this.bound_handler);
        }
    }
}
