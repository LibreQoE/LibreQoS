import assert from "node:assert/strict";
import test from "node:test";

import {
    bridgeEligibilityField,
    normalizedBridgeFlags,
    saveManualXdpConfiguration,
    usesManualXdpWorkflow,
} from "./network_mode_shim.mjs";

test("compatibility shim selects relaxed bridge eligibility", () => {
    assert.equal(bridgeEligibilityField("bridge", false), "bridge_eligible");
    assert.equal(bridgeEligibilityField("bridge", true), "compatibility_shim_eligible");
    assert.equal(bridgeEligibilityField("single", true), "single_interface_eligible");
});

test("compatibility shim always enables the XDP bridge", () => {
    assert.deepEqual(normalizedBridgeFlags(false, true), {
        use_xdp_bridge: true,
        compatibility_shim: true,
    });
    assert.deepEqual(normalizedBridgeFlags(false, false), {
        use_xdp_bridge: false,
        compatibility_shim: false,
    });
});

test("XDP and compatibility-shim configs use config-only saves", () => {
    assert.equal(usesManualXdpWorkflow({ bridge: { use_xdp_bridge: true } }), true);
    assert.equal(usesManualXdpWorkflow({ bridge: { use_xdp_bridge: false } }), false);
    assert.equal(usesManualXdpWorkflow({ single_interface: {} }), false);
});

test("manual XDP save completes the page workflow and keeps the saved candidate", async () => {
    const previousConfig = { bridge: { use_xdp_bridge: false } };
    const candidate = { bridge: { use_xdp_bridge: true, compatibility_shim: true } };
    const configState = { config: previousConfig };
    let savedConfig = null;
    const effects = [];

    const result = await saveManualXdpConfiguration({
        candidate,
        configState,
        persistConfig: (onSuccess) => {
            savedConfig = configState.config;
            effects.push("persist");
            onSuccess({ ok: true });
        },
        clearDraft: () => effects.push("clear-draft"),
        notifySaved: () => effects.push("notify"),
        refresh: async () => effects.push("refresh"),
    });

    assert.equal(savedConfig, candidate);
    assert.equal(configState.config, candidate);
    assert.deepEqual(effects, ["persist", "clear-draft", "notify", "refresh"]);
    assert.equal(result.refreshError, null);
});

test("manual XDP save restores config and skips success effects after failure", async () => {
    const previousConfig = { bridge: { use_xdp_bridge: false } };
    const candidate = { bridge: { use_xdp_bridge: true, compatibility_shim: true } };
    const expectedError = new Error("save failed");
    const configState = { config: previousConfig };
    const effects = [];

    await assert.rejects(
        saveManualXdpConfiguration({
            candidate,
            configState,
            persistConfig: (_onSuccess, onError) => {
                assert.equal(configState.config, candidate);
                effects.push("persist");
                onError(expectedError);
            },
            clearDraft: () => effects.push("clear-draft"),
            notifySaved: () => effects.push("notify"),
            refresh: async () => effects.push("refresh"),
        }),
        expectedError,
    );

    assert.equal(configState.config, previousConfig);
    assert.deepEqual(effects, ["persist"]);
});

test("manual XDP save keeps saved config when the status refresh fails", async () => {
    const previousConfig = { bridge: { use_xdp_bridge: false } };
    const candidate = { bridge: { use_xdp_bridge: true, compatibility_shim: true } };
    const configState = { config: previousConfig };
    const refreshError = new Error("refresh failed");
    const effects = [];

    const result = await saveManualXdpConfiguration({
        candidate,
        configState,
        persistConfig: (onSuccess) => {
            effects.push("persist");
            onSuccess({ ok: true });
        },
        clearDraft: () => effects.push("clear-draft"),
        notifySaved: () => effects.push("notify"),
        refresh: async () => {
            effects.push("refresh");
            throw refreshError;
        },
    });

    assert.equal(configState.config, candidate);
    assert.equal(result.refreshError, refreshError);
    assert.deepEqual(effects, ["persist", "clear-draft", "notify", "refresh"]);
});
