import assert from "node:assert/strict";
import test from "node:test";

globalThis.window = {
    LQOS_UI_VERSION: "test",
    location: { search: "", protocol: "http:", host: "localhost" },
    localStorage: { getItem: () => null },
    sessionStorage: {
        getItem: () => null,
        setItem: () => {},
        removeItem: () => {},
    },
};
globalThis.document = { cookie: "" };

const { listenOnceMatchingWithTimeout } = await import("./listeners.js");

function makeWsClient() {
    const handlers = new Map();
    return {
        on(eventName, handler) {
            if (!handlers.has(eventName)) {
                handlers.set(eventName, new Set());
            }
            handlers.get(eventName).add(handler);
            return () => this.off(eventName, handler);
        },
        off(eventName, handler) {
            handlers.get(eventName)?.delete(handler);
        },
        emit(eventName, message) {
            for (const handler of Array.from(handlers.get(eventName) || [])) {
                handler(message);
            }
        },
        handlerCount(eventName) {
            return handlers.get(eventName)?.size || 0;
        },
    };
}

test("timeout listener ignores stale responses and resolves matching response", () => {
    const wsClient = makeWsClient();
    const seen = [];

    listenOnceMatchingWithTimeout(
        wsClient,
        "UrgentClearResult",
        1000,
        (message) => message.request_id === 2,
        (message) => seen.push(message.ok),
        () => seen.push("timeout"),
    );

    wsClient.emit("UrgentClearResult", { request_id: 1, ok: false });
    assert.deepEqual(seen, []);
    assert.equal(wsClient.handlerCount("UrgentClearResult"), 1);

    wsClient.emit("UrgentClearResult", { request_id: 2, ok: true });
    assert.deepEqual(seen, [true]);
    assert.equal(wsClient.handlerCount("UrgentClearResult"), 0);
});

test("timeout listener disposer removes pending handler", () => {
    const wsClient = makeWsClient();
    const seen = [];

    const dispose = listenOnceMatchingWithTimeout(
        wsClient,
        "UrgentList",
        1000,
        () => true,
        () => seen.push("response"),
        () => seen.push("timeout"),
    );

    assert.equal(wsClient.handlerCount("UrgentList"), 1);
    dispose();
    wsClient.emit("UrgentList", { request_id: 1 });

    assert.deepEqual(seen, []);
    assert.equal(wsClient.handlerCount("UrgentList"), 0);
});

test("timeout listener fires timeout and removes pending handler", async () => {
    const wsClient = makeWsClient();
    const seen = [];

    listenOnceMatchingWithTimeout(
        wsClient,
        "UrgentList",
        5,
        () => false,
        () => seen.push("response"),
        () => seen.push("timeout"),
    );

    assert.equal(wsClient.handlerCount("UrgentList"), 1);
    await new Promise((resolve) => setTimeout(resolve, 20));

    assert.deepEqual(seen, ["timeout"]);
    assert.equal(wsClient.handlerCount("UrgentList"), 0);
});

test("websocket handshake does not copy the session cookie into auth payload", async () => {
    let cookieRead = false;
    Object.defineProperty(globalThis.document, "cookie", {
        configurable: true,
        get() {
            cookieRead = true;
            return "User-Token=v1.secret.signature";
        },
    });

    const { websocketHelloReply } = await import("./ws_auth.mjs");
    const reply = websocketHelloReply("ack");

    assert.equal(cookieRead, false);
    assert.equal(reply.HelloReply.token, "");
});
