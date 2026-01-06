import { get_ws_client, resetWS as reset_shared } from "../../pubsub/ws";

export function subscribeWS(channels, handler) {
    if (!channels || channels.length === 0) {
        return;
    }
    const client = get_ws_client();
    client.on("join", handler);
    for (let i = 0; i < channels.length; i++) {
        client.on(channels[i], handler);
    }
    client.subscribe(channels);
}

export function resetWS() {
    reset_shared();
}
