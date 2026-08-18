import { Encoder, decode } from "../lq_js_common/helpers/cbor-x";
import { websocketHelloReply } from "./ws_auth.mjs";

const ACK_TEXT = "I accept that this is an unstable, internal API and is unsupported";
const EXPECTED_UI_VERSION = (window.LQOS_UI_VERSION || "").trim() || null;
const VERSION_RELOAD_KEY = "lqosWsVersionReload";
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

function versionReloadKey(serverVersion) {
    const clientVersion = EXPECTED_UI_VERSION || "missing";
    const wsVersion = (serverVersion || "missing").toString().trim() || "missing";
    return `${clientVersion}->${wsVersion}`;
}

function hasReloadedForVersionMismatch(mismatchKey) {
    try {
        if (window.sessionStorage.getItem(VERSION_RELOAD_KEY) === mismatchKey) {
            return true;
        }
        window.sessionStorage.setItem(VERSION_RELOAD_KEY, mismatchKey);
        return false;
    } catch (_) {
        if (window.__lqosWsVersionReload === mismatchKey) {
            return true;
        }
        window.__lqosWsVersionReload = mismatchKey;
        return false;
    }
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
        this.versionReloading = false;
    }

    connect() {
        if (this.ws) {
            if (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN) {
                return;
            }
            this._dropSocket("stale-socket");
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
            if (!this.versionReloading) {
                this._scheduleReconnect();
            }
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
            try {
                socket.close();
            } catch (_) {
                // The browser may already have closed the socket.
            }
            if (!this.versionReloading) {
                this._scheduleReconnect();
            }
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

    _dropSocket(reason = "unspecified") {
        dashboardWsDebug("drop-socket", {
            reason,
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        const oldSocket = this.ws;
        this.ws = null;
        this.handshake_done = false;
        if (oldSocket) {
            try {
                oldSocket.close();
            } catch (_) {
                // Ignore close errors; reconnect/reload handling continues below.
            }
        }
    }

    _reloadForVersionMismatch(serverVersion) {
        const mismatchKey = versionReloadKey(serverVersion);
        dashboardWsDebug("version-mismatch", {
            expected: EXPECTED_UI_VERSION || "missing",
            actual: serverVersion || "missing",
            mismatchKey,
        });

        if (!hasReloadedForVersionMismatch(mismatchKey)) {
            this.versionReloading = true;
            this._dropSocket("version-mismatch");
            window.location.reload();
            window.setTimeout(() => {
                if (this.versionReloading) {
                    this.versionReloading = false;
                    this._scheduleReconnect();
                }
            }, 3000);
            return true;
        }

        console.error(
            "Websocket version mismatch persisted after reload:",
            `client=${EXPECTED_UI_VERSION || "missing"}`,
            `server=${serverVersion || "missing"}`,
        );
        this._dropSocket("version-mismatch-persistent");
        this._scheduleReconnect();
        return true;
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
                this._sendControl({ Subscribe: { channel } });
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
                    this._sendControl({ Unsubscribe: { channel } });
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
        if (!this.handshake_done || !this._socketIsOpen()) {
            this.pending.push(normalized);
            if (this.handshake_done && this.ws && !this._socketIsOpen()) {
                this._retireSocket();
            }
            return;
        }
        if (!this._sendControl(normalized)) {
            this.pending.push(normalized);
        }
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
            this._reloadForVersionMismatch(hello ? hello.version : null);
            return;
        }
        try {
            window.sessionStorage.removeItem(VERSION_RELOAD_KEY);
        } catch (_) {
            // Storage cleanup is best-effort.
        }
        this.handshake_done = true;
        dashboardWsDebug("handshake-complete", {
            desiredChannels: Array.from(this.desiredChannels.keys()),
        });
        if (!this._sendControl(websocketHelloReply(ACK_TEXT))) {
            return;
        }
        const pending = this.pending;
        this.pending = [];
        for (let i = 0; i < pending.length; i++) {
            if (!this._sendControl(pending[i])) {
                this.pending = pending.slice(i).concat(this.pending);
                return;
            }
        }
        for (const channel of this.desiredChannels.keys()) {
            dashboardWsInteresting("subscribe-wire-interesting", [channel], {
                desiredChannels: Array.from(this.desiredChannels.keys()),
                fromHandshake: true,
            });
            if (!this._sendControl({ Subscribe: { channel } })) {
                return;
            }
        }
        this.reconnectDelayMs = 1000;
    }

    _socketIsOpen() {
        return this.ws && this.ws.readyState === WebSocket.OPEN;
    }

    _retireSocket() {
        this._dropSocket("retire-socket");
        this._scheduleReconnect();
    }

    _sendControl(request_obj) {
        if (!this._socketIsOpen()) {
            this._retireSocket();
            return false;
        }
        try {
            this.ws.send(encoder.encode(request_obj));
            return true;
        } catch (err) {
            console.warn("Websocket send failed; reconnecting", err);
            this._retireSocket();
            return false;
        }
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
            try {
                handler(msg);
            } catch (err) {
                console.error(`Websocket handler failed for ${msg.event}`, err);
                dashboardWsDebug("handler-error", {
                    eventName: msg.event,
                    message: err && err.message ? err.message : String(err),
                });
            }
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
