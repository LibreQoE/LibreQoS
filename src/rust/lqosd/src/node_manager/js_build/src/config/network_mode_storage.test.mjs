import assert from "node:assert/strict";
import test from "node:test";

import {
    clearOperationalNetworkModeStorage,
    DRAFT_KEY,
    loadNetworkModeState,
    PENDING_OPERATION_KEY,
    saveNetworkModeState,
} from "./network_mode_storage.mjs";

class FakeStorage {
    constructor(entries = {}) {
        this.values = new Map(Object.entries(entries));
    }

    getItem(key) {
        return this.values.has(key) ? this.values.get(key) : null;
    }

    setItem(key, value) {
        this.values.set(key, String(value));
    }

    removeItem(key) {
        this.values.delete(key);
    }
}

function withStorage({localEntries = {}, sessionEntries = {}}, callback) {
    const previousWindow = globalThis.window;
    const localStorage = new FakeStorage(localEntries);
    const sessionStorage = new FakeStorage(sessionEntries);

    globalThis.window = {localStorage, sessionStorage};
    try {
        callback({localStorage, sessionStorage});
    } finally {
        if (previousWindow === undefined) {
            delete globalThis.window;
        } else {
            globalThis.window = previousWindow;
        }
    }
}

test("saves network mode draft state only in tab storage", () => {
    withStorage({
        localEntries: {
            [DRAFT_KEY]: JSON.stringify({mode: "legacy"}),
        },
    }, ({localStorage, sessionStorage}) => {
        saveNetworkModeState(DRAFT_KEY, {mode: "bridge"});

        assert.deepEqual(loadNetworkModeState(DRAFT_KEY), {mode: "bridge"});
        assert.equal(sessionStorage.getItem(DRAFT_KEY), JSON.stringify({mode: "bridge"}));
        assert.equal(localStorage.getItem(DRAFT_KEY), null);
    });
});

test("migrates existing localStorage drafts into tab storage once", () => {
    withStorage({
        localEntries: {
            [DRAFT_KEY]: JSON.stringify({mode: "single_interface", interface: "eth0"}),
        },
    }, ({localStorage, sessionStorage}) => {
        assert.deepEqual(loadNetworkModeState(DRAFT_KEY), {mode: "single_interface", interface: "eth0"});
        assert.equal(sessionStorage.getItem(DRAFT_KEY), JSON.stringify({mode: "single_interface", interface: "eth0"}));
        assert.equal(localStorage.getItem(DRAFT_KEY), null);
    });
});

test("drops malformed legacy network mode state", () => {
    withStorage({
        localEntries: {
            [PENDING_OPERATION_KEY]: "{not-json",
        },
    }, ({localStorage, sessionStorage}) => {
        assert.equal(loadNetworkModeState(PENDING_OPERATION_KEY), null);
        assert.equal(sessionStorage.getItem(PENDING_OPERATION_KEY), null);
        assert.equal(localStorage.getItem(PENDING_OPERATION_KEY), null);
    });
});

test("clears operational network mode state without touching durable preferences", () => {
    withStorage({
        localEntries: {
            [DRAFT_KEY]: JSON.stringify({mode: "legacy"}),
            "lqos-theme": "dark",
        },
        sessionEntries: {
            [DRAFT_KEY]: JSON.stringify({mode: "bridge"}),
            [PENDING_OPERATION_KEY]: JSON.stringify({operation_id: "op-1"}),
        },
    }, ({localStorage, sessionStorage}) => {
        clearOperationalNetworkModeStorage();

        assert.equal(sessionStorage.getItem(DRAFT_KEY), null);
        assert.equal(sessionStorage.getItem(PENDING_OPERATION_KEY), null);
        assert.equal(localStorage.getItem(DRAFT_KEY), null);
        assert.equal(localStorage.getItem("lqos-theme"), "dark");
    });
});
