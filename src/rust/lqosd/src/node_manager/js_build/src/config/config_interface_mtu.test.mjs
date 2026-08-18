import assert from "node:assert/strict";
import test from "node:test";

import {hydrateDraftMtu, parseOptionalMtu} from "./network_mode_mtu.mjs";

test("accepts blank and valid MTU values", () => {
    assert.deepEqual(parseOptionalMtu(""), { ok: true, value: null, error: "" });
    assert.deepEqual(parseOptionalMtu("  "), { ok: true, value: null, error: "" });
    assert.deepEqual(parseOptionalMtu("1500"), { ok: true, value: 1500, error: "" });
    assert.deepEqual(parseOptionalMtu("9000"), { ok: true, value: 9000, error: "" });
});

test("rejects invalid MTU values", () => {
    assert.equal(parseOptionalMtu("abc").ok, false);
    assert.equal(parseOptionalMtu("1500.5").ok, false);
    assert.equal(parseOptionalMtu("575").ok, false);
    assert.equal(parseOptionalMtu("9217").ok, false);
});

test("returns normalized candidate values", () => {
    assert.equal(parseOptionalMtu("9000").value, 9000);
    assert.equal(parseOptionalMtu("").value, null);
});

test("hydrates pre-MTU drafts from live config", () => {
    const liveConfig = {
        bridge: { mtu: 9000 },
        single_interface: { mtu: 1500 },
    };

    assert.equal(hydrateDraftMtu({ bridge: { to_internet: "eth0" } }, liveConfig).bridge.mtu, 9000);
    assert.equal(
        hydrateDraftMtu({ single_interface: { interface: "eth0" } }, liveConfig).single_interface.mtu,
        1500
    );
});

test("keeps explicit draft MTU values", () => {
    const liveConfig = {
        bridge: { mtu: 9000 },
        single_interface: { mtu: 1500 },
    };

    assert.equal(hydrateDraftMtu({ bridge: { mtu: null } }, liveConfig).bridge.mtu, null);
    assert.equal(hydrateDraftMtu({ single_interface: { mtu: 1400 } }, liveConfig).single_interface.mtu, 1400);
});
