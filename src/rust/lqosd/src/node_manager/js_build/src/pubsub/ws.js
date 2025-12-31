import { Encoder, decode } from "../lq_js_common/helpers/cbor-x";

const ACK_TEXT = "I accept that this is an unstable, internal API and is unsupported";
const EXPECTED_UI_VERSION = (window.LQOS_UI_VERSION || "").trim() || null;
const USER_TOKEN_COOKIE = "User-Token";
const encoder = new Encoder({ useRecords: false, variableMapSize: true });

let shared_client = null;

function get_cookie_value(name) {
    const cookies = document.cookie ? document.cookie.split(";") : [];
    const prefix = `${name}=`;
    for (let i = 0; i < cookies.length; i++) {
        const entry = cookies[i].trim();
        if (entry.startsWith(prefix)) {
            return decodeURIComponent(entry.slice(prefix.length));
        }
    }
    return "";
}

function get_user_token() {
    return get_cookie_value(USER_TOKEN_COOKIE);
}

export function ws_proto() {
    if (window.location.protocol.startsWith("https")) {
        return "wss://";
    }
    return "ws://";
}

export class WsClient {
    constructor() {
        this.ws = null;
        this.handlers = new Map();
        this.pending = [];
        this.handshake_done = false;
    }

    connect() {
        if (this.ws) {
            return;
        }
        this.ws = new WebSocket(ws_proto() + window.location.host + "/websocket/ws");
        this.ws.binaryType = "arraybuffer";

        this.ws.onmessage = async (event) => {
            let msg = null;
            try {
                if (event.data instanceof ArrayBuffer) {
                    msg = decode(new Uint8Array(event.data));
                } else if (event.data && typeof event.data.arrayBuffer === "function") {
                    const buf = await event.data.arrayBuffer();
                    msg = decode(new Uint8Array(buf));
                }
            } catch (err) {
                console.error("Failed to decode websocket message", err);
                return;
            }

            if (msg && msg.event === "Hello") {
                this._acknowledge_handshake(msg);
                return;
            }
            this._dispatch(msg);
        };

        this.ws.onclose = () => {
            this.ws = null;
            this.handshake_done = false;
            this.pending = [];
        };

        this.ws.onerror = () => {
            this.ws = null;
            this.handshake_done = false;
        };
    }

    close() {
        if (this.ws) {
            this.ws.close();
        }
        this.ws = null;
        this.handshake_done = false;
        this.pending = [];
    }

    subscribe(channels) {
        for (let i = 0; i < channels.length; i++) {
            this.send({ Subscribe: { channel: channels[i] } });
        }
    }

    send(request_obj) {
        const normalized = normalizeRequest(request_obj);
        if (!this.ws) {
            this.connect();
        }
        if (!this.handshake_done) {
            this.pending.push(normalized);
            return;
        }
        this.ws.send(encoder.encode(normalized));
    }

    on(event_name, handler) {
        const list = this.handlers.get(event_name) || [];
        list.push(handler);
        this.handlers.set(event_name, list);
    }

    off(event_name, handler) {
        const list = this.handlers.get(event_name);
        if (!list) {
            return;
        }
        this.handlers.set(
            event_name,
            list.filter((h) => h !== handler),
        );
    }

    _acknowledge_handshake(hello) {
        if (this.handshake_done) {
            return;
        }
        if (!EXPECTED_UI_VERSION || hello.version !== EXPECTED_UI_VERSION) {
            console.error(
                "Websocket version mismatch:",
                hello ? hello.version : "missing",
            );
            this.close();
            return;
        }
        this.handshake_done = true;
        this.ws.send(
            encoder.encode({
                HelloReply: {
                    ack: ACK_TEXT,
                    token: get_user_token(),
                },
            }),
        );
        for (let i = 0; i < this.pending.length; i++) {
            this.ws.send(encoder.encode(this.pending[i]));
        }
        this.pending = [];
    }

    _dispatch(msg) {
        if (!msg || !msg.event) {
            return;
        }
        const list = this.handlers.get(msg.event);
        if (!list) {
            return;
        }
        for (let i = 0; i < list.length; i++) {
            list[i](msg);
        }
    }
}

function normalizeRequest(request_obj) {
    if (!request_obj || typeof request_obj !== "object") {
        return request_obj;
    }
    const keys = Object.keys(request_obj);
    if (keys.length !== 1) {
        return request_obj;
    }
    const key = keys[0];
    const value = request_obj[key];
    if (
        value &&
        typeof value === "object" &&
        !Array.isArray(value) &&
        Object.keys(value).length === 0
    ) {
        return { [key]: null };
    }
    return request_obj;
}

export function get_ws_client() {
    if (typeof window !== "undefined") {
        if (!window.__lqos_ws_client) {
            window.__lqos_ws_client = new WsClient();
        }
        shared_client = window.__lqos_ws_client;
        return window.__lqos_ws_client;
    }
    if (!shared_client) {
        shared_client = new WsClient();
    }
    return shared_client;
}

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
    if (shared_client) {
        shared_client.close();
    }
    if (typeof window !== "undefined" && window.__lqos_ws_client) {
        window.__lqos_ws_client.close();
        delete window.__lqos_ws_client;
    }
    shared_client = null;
}
