import assert from "node:assert/strict";
import test from "node:test";

import {
    TOPOLOGY_SOURCES,
    buildTopologyConfigUpdate,
    topologyEditorsLockMessage,
    topologyEditorsLocked,
} from "./config/topology_source.js";

globalThis.window = { configView: {} };

const integrationSources = TOPOLOGY_SOURCES.filter((entry) => entry.integration);

test("the lock matrix is source-driven", () => {
    for (const entry of integrationSources) {
        const config = { topology: { source: entry.value } };
        assert.equal(topologyEditorsLocked(config), true, entry.value);
        assert.match(topologyEditorsLockMessage(config), new RegExp(entry.label));
    }

    assert.equal(
        topologyEditorsLocked({ topology: { source: "libreqos_network" } }),
        true,
    );
    assert.match(
        topologyEditorsLockMessage({ topology: { source: "libreqos_network" } }),
        /libreqos_network\.json drop/,
    );

    assert.equal(
        topologyEditorsLocked({ topology: { source: "shaped_devices" } }),
        false,
    );
    assert.equal(
        topologyEditorsLockMessage({ topology: { source: "shaped_devices" } }),
        "",
    );

    assert.equal(topologyEditorsLocked({ topology: {} }), true);
    assert.match(topologyEditorsLockMessage({ topology: {} }), /No topology source/);
});

test("a shaped_devices conflict re-locks the editors and is named", () => {
    window.configView = {
        topology_source_conflict: {
            message: "topology source conflict: enable_uisp is enabled",
        },
    };
    const config = { topology: { source: "shaped_devices" } };
    assert.equal(topologyEditorsLocked(config), true);
    assert.match(topologyEditorsLockMessage(config), /enable_uisp/);
    window.configView = {};
});

test("the save payload contains exactly the selector and mode, not the flags", () => {
    const config = {
        topology: { source: "uisp", compile_mode: "flat" },
        uisp_integration: { enable_uisp: true, token: "keep" },
        visp_integration: { enable_visp: true, client_secret: "keep-visp" },
    };
    const updated = buildTopologyConfigUpdate(config, "visp", "full");
    assert.equal(updated.source, "visp");
    assert.equal(updated.compile_mode, "full");
    assert.equal(config.uisp_integration.enable_uisp, true);
    assert.equal(config.visp_integration.enable_visp, true);
    assert.equal(config.uisp_integration.token, "keep");
});
