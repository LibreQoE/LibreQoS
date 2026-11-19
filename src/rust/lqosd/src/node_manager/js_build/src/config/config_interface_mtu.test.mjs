import assert from "node:assert/strict";
import test from "node:test";

import {hydrateDraftMtu, MTU_MAX, MTU_MIN, parseOptionalMtu} from "./network_mode_mtu.mjs";
import {clearCatalogs, EN, ES, installCatalogs, installKeyEchoCatalogs} from "../../i18n/test_catalogs.mjs";

test.beforeEach(() => {
    installCatalogs();
});

test.afterEach(() => {
    clearCatalogs();
});

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

test("every MTU error maps to a key both catalogs define", () => {
    installKeyEchoCatalogs();
    for (const raw of ["abc", "575", "9217"]) {
        const error = parseOptionalMtu(raw).error;
        assert.equal(typeof EN.messages[error], "string", `${error} is missing from en.json`);
        assert.equal(typeof ES.messages[error], "string", `${error} is missing from es.json`);
    }
});

test("the English MTU errors are unchanged", () => {
    assert.equal(parseOptionalMtu("abc").error, "MTU must be a whole number from 576 through 9216.");
    assert.equal(parseOptionalMtu("9217").error, "MTU must be from 576 through 9216.");
});

test("the Spanish MTU errors keep the bounds literal", () => {
    installCatalogs(ES);
    // 9216 must not be grouped as "9.216": an MTU is a literal, not a quantity.
    assert.equal(
        parseOptionalMtu("abc").error,
        `La MTU debe ser un número entero de ${MTU_MIN} a ${MTU_MAX}.`,
    );
    assert.equal(parseOptionalMtu("9217").error, `La MTU debe estar entre ${MTU_MIN} y ${MTU_MAX}.`);
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
