import { get_ws_client } from "./ws";

export class DirectChannel {
    constructor(subObject, handler) {
        this.client = get_ws_client();
        this.handler = handler;
        this.event_name = Object.keys(subObject)[0];
        this.bound_handler = (msg) => {
            handler(msg);
        };
        this.client.on(this.event_name, this.bound_handler);
        this.client.send({ Private: subObject });
    }

    close() {
        if (!this.client || !this.bound_handler) {
            return;
        }
        this.client.off(this.event_name, this.bound_handler);
    }
}
