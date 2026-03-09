import { get_ws_client, resetWS as reset_shared } from "../../pubsub/ws";

export function subscribeWS(channels, handler) {
    if (!channels || channels.length === 0) {
        return { dispose() {} };
    }
    const client = get_ws_client();
    const disposers = [];
    disposers.push(client.on("join", handler));
    for (let i = 0; i < channels.length; i++) {
        disposers.push(client.on(channels[i], handler));
    }
    client.subscribe(channels);
    return {
        dispose() {
            for (let i = 0; i < disposers.length; i++) {
                disposers[i]();
            }
            client.unsubscribe(channels);
        },
    };
}

export function resetWS() {
    reset_shared();
}
