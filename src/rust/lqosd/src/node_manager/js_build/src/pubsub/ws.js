import { Encoder, decode } from "../lq_js_common/helpers/cbor-x";

const ACK_TEXT = "I accept that this is an unstable, internal API and is unsupported";
const EXPECTED_UI_VERSION = (window.LQOS_UI_VERSION || "").trim() || null;
const USER_TOKEN_COOKIE = "User-Token";
const encoder = new Encoder({ useRecords: false, variableMapSize: true });
const DIAGNOSTIC_CHANNELS = new Set(["Cpu", "Ram", "RttHistogram"]);

let shared_client = null;

function dashboardDebugEnabled() {
    if (typeof window === "undefined") {
        return false;
    }
    try {
        const params = new URLSearchParams(window.location.search || "");
        if (params.has("debug")) {
            const value = (params.get("debug") || "").trim().toLowerCase();
            if (value === "" || value === "1" || value === "true" || value === "dashboard") {
                return true;
            }
        }
    } catch (_) {
        // Ignore malformed URL search state and fall back to localStorage.
    }
    return !!window.localStorage && window.localStorage.getItem("debugDashboard") === "1";
}

function pushDashboardWsTrace(entry) {
    if (typeof window === "undefined") {
        return;
    }
    if (!window.__lqosDashboardWsTrace) {
        window.__lqosDashboardWsTrace = [];
    }
    window.__lqosDashboardWsTrace.push({
        ts: new Date().toISOString(),
        ...entry,
    });
    if (window.__lqosDashboardWsTrace.length > 500) {
        window.__lqosDashboardWsTrace.shift();
    }
}

function dashboardWsDebug(event, details = {}) {
    if (!dashboardDebugEnabled()) {
        return;
    }
    const entry = { event, ...details };
    pushDashboardWsTrace(entry);
    console.debug("[dashboard-ws]", entry);
}

function dashboardWsInteresting(event, channels = [], details = {}) {
    const interesting = channels.filter((channel) => DIAGNOSTIC_CHANNELS.has(channel));
    if (interesting.length === 0) {
        return;
    }
    dashboardWsDebug(event, {
        channels: interesting,
        ...details,
    });
}

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
        this.desiredChannels = new Map();
        this.handshake_done = false;
        this.reconnectTimer = null;
        this.reconnectDelayMs = 1000;
        this.manualClose = false;
    }

    connect() {
        if (this.ws) {
            return;
        }
        if (this.reconnectTimer) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = null;
        }
        this.manualClose = false;
        const socket = new WebSocket(ws_proto() + window.location.host + "/websocket/ws");
        this.ws = socket;
        socket.binaryType = "arraybuffer";
        dashboardWsDebug("connect", {
            url: ws_proto() + window.location.host + "/websocket/ws",
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });

        socket.onmessage = async (event) => {
            if (this.ws !== socket) {
                return;
            }
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
                dashboardWsDebug("hello", {
                    version: msg.version,
                });
                this._acknowledge_handshake(msg);
                return;
            }
            dashboardWsDebug("message", {
                eventName: msg && msg.event ? msg.event : "unknown",
            });
            if (msg && msg.event && DIAGNOSTIC_CHANNELS.has(msg.event)) {
                dashboardWsDebug("message-interesting", {
                    eventName: msg.event,
                    desiredChannels: Array.from(this.desiredChannels.keys()),
                });
            }
            this._dispatch(msg);
        };

        socket.onclose = () => {
            if (this.ws !== socket) {
                return;
            }
            dashboardWsDebug("close", {
                desiredChannels: Array.from(this.desiredChannels.keys()),
            });
            this.ws = null;
            this.handshake_done = false;
            this._scheduleReconnect();
        };

        socket.onerror = () => {
            if (this.ws !== socket) {
                return;
            }
            dashboardWsDebug("error", {
                desiredChannels: Array.from(this.desiredChannels.keys()),
            });
            this.ws = null;
            this.handshake_done = false;
            this._scheduleReconnect();
        };
    }

    close() {
        dashboardWsDebug("manual-close", {
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        this.manualClose = true;
        if (this.reconnectTimer) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = null;
        }
        if (this.ws) {
            this.ws.close();
        }
        this.ws = null;
        this.handshake_done = false;
        this.pending = [];
        this.desiredChannels.clear();
        this.handlers.clear();
    }

    refreshSubscriptions(reason = "unspecified") {
        dashboardWsDebug("refresh-subscriptions", {
            reason,
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        if (this.reconnectTimer) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = null;
        }
        const oldSocket = this.ws;
        this.ws = null;
        this.handshake_done = false;
        if (oldSocket) {
            try {
                oldSocket.close();
            } catch (_) {
                // Ignore close errors; a new socket is opened below if needed.
            }
        }
        if (this._shouldMaintainConnection()) {
            this.connect();
        }
    }

    subscribe(channels) {
        dashboardWsDebug("subscribe-request", {
            channels,
        });
        dashboardWsInteresting("subscribe-request-interesting", channels, {
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        for (let i = 0; i < channels.length; i++) {
            const channel = channels[i];
            const current = this.desiredChannels.get(channel) || 0;
            this.desiredChannels.set(channel, current + 1);
            if (current === 0 && this.handshake_done && this.ws) {
                dashboardWsDebug("subscribe-wire", {
                    channel,
                });
                dashboardWsInteresting("subscribe-wire-interesting", [channel], {
                    desiredChannels: Array.from(this.desiredChannels.keys()),
                });
                this.ws.send(encoder.encode({ Subscribe: { channel } }));
            }
        }
        if (channels.length > 0) {
            this.connect();
        }
    }

    unsubscribe(channels) {
        dashboardWsDebug("unsubscribe-request", {
            channels,
        });
        dashboardWsInteresting("unsubscribe-request-interesting", channels, {
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        for (let i = 0; i < channels.length; i++) {
            const channel = channels[i];
            const current = this.desiredChannels.get(channel) || 0;
            if (current <= 1) {
                this.desiredChannels.delete(channel);
                if (this.handshake_done && this.ws) {
                    dashboardWsDebug("unsubscribe-wire", {
                        channel,
                    });
                    dashboardWsInteresting("unsubscribe-wire-interesting", [channel], {
                        desiredChannels: Array.from(this.desiredChannels.keys()),
                    });
                    this.ws.send(encoder.encode({ Unsubscribe: { channel } }));
                }
            } else {
                this.desiredChannels.set(channel, current - 1);
            }
        }
    }

    send(request_obj) {
        const normalized = normalizeRequest(request_obj);
        dashboardWsDebug("send", {
            request: normalized && typeof normalized === "object" ? Object.keys(normalized)[0] : "unknown",
        });
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
        let list = this.handlers.get(event_name);
        if (!list) {
            list = new Set();
            this.handlers.set(event_name, list);
        }
        list.add(handler);
        return () => {
            this.off(event_name, handler);
        };
    }

    off(event_name, handler) {
        const list = this.handlers.get(event_name);
        if (!list) {
            return;
        }
        list.delete(handler);
        if (list.size === 0) {
            this.handlers.delete(event_name);
        }
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
        dashboardWsDebug("handshake-complete", {
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
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
        for (const channel of this.desiredChannels.keys()) {
            dashboardWsInteresting("subscribe-wire-interesting", [channel], {
                desiredChannels: Array.from(this.desiredChannels.keys()),
                fromHandshake: true,
            });
            this.ws.send(encoder.encode({ Subscribe: { channel } }));
        }
        this.reconnectDelayMs = 1000;
    }

    _dispatch(msg) {
        if (!msg || !msg.event) {
            return;
        }
        const list = this.handlers.get(msg.event);
        if (!list) {
            return;
        }
        for (const handler of Array.from(list)) {
            handler(msg);
        }
    }

    _scheduleReconnect() {
        if (this.manualClose || this.reconnectTimer) {
            return;
        }
        if (!this._shouldMaintainConnection()) {
            return;
        }
        const delay = this.reconnectDelayMs;
        dashboardWsDebug("schedule-reconnect", {
            delayMs: delay,
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = null;
            this.connect();
        }, delay);
        this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 10000);
    }

    _shouldMaintainConnection() {
        return (
            this.pending.length > 0 ||
            this.desiredChannels.size > 0 ||
            this.handlers.size > 0
        );
    }
}

function normalizeRequest(request_obj) {
    if (!request_obj || typeof request_obj !== "object") {
        return request_obj;
    }
    const stripUndefined = (value) => {
        if (Array.isArray(value)) {
            return value.map((entry) =>
                entry === undefined ? null : stripUndefined(entry),
            );
        }
        if (!value || typeof value !== "object") {
            return value;
        }
        const result = {};
        Object.keys(value).forEach((key) => {
            const entry = value[key];
            if (entry === undefined) {
                return;
            }
            result[key] = stripUndefined(entry);
        });
        return result;
    };

    const keys = Object.keys(request_obj);
    if (keys.length !== 1) {
        return stripUndefined(request_obj);
    }
    const key = keys[0];
    const value = stripUndefined(request_obj[key]);
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
    if (shared_client) {
        shared_client.close();
    }
    if (typeof window !== "undefined" && window.__lqos_ws_client) {
        window.__lqos_ws_client.close();
        delete window.__lqos_ws_client;
    }
    shared_client = null;
}
