import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import test from "node:test";

// `TOPOLOGY_SOURCES` renders its labels and descriptions through the catalog
// when the module loads, so the English catalog has to be on `window` first:
// static imports run before any statement here, hence the dynamic import.
const EN = JSON.parse(readFileSync(new URL("../../static2/i18n/en.json", import.meta.url), "utf8"));
globalThis.window = { configView: {}, LQOS_I18N_EN: EN };

const {
    TOPOLOGY_SOURCES,
    buildTopologyConfigUpdate,
    topologyEditorsLockMessage,
    topologyEditorsLocked,
} = await import("./config/topology_source.js");

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
