import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";

const sourceDirectory = new URL(".", import.meta.url);
const helperPath = fileURLToPath(new URL("config/config_helper.js", sourceDirectory));
const topologyPath = fileURLToPath(new URL("config/topology_source.js", sourceDirectory));
const pagePath = fileURLToPath(new URL("../../static2/config_integration.html", sourceDirectory));
const scriptPath = fileURLToPath(new URL("config_integration.js", sourceDirectory));

test("topology source selector is the one authority", async () => {
    const [helper, topology, page, script] = await Promise.all([
        readFile(helperPath, "utf8"),
        readFile(topologyPath, "utf8"),
        readFile(pagePath, "utf8"),
        readFile(scriptPath, "utf8"),
    ]);

    assert.match(topology, /export const TOPOLOGY_SOURCES/);
    assert.match(topology, /export function topologyEditorsLocked/);
    assert.match(topology, /!== "shaped_devices"|=== "shaped_devices"/);
    assert.match(topology, /export function topologyEditorsLockMessage/);
    assert.match(topology, /No topology source is selected/);
    assert.match(topology, /libreqos_network\.json drop/);
    assert.match(helper, /export function applyTopologySourceCheckboxState/);
    assert.match(helper, /from "\.\/topology_source"/);
    assert.match(page, /id="topologySource"/);
    assert.match(page, /id="topologySourceStatus"/);
    assert.match(page, /id="topologySourceConflict"/);
    assert.match(script, /buildTopologyConfigUpdate\(/);
    assert.match(script, /document\.getElementById\("topologySource"\)\.value/);
    assert.match(script, /definition\.modes\.includes\(topologyMode\)/);
    assert.match(script, /last_reject_reason/);
    assert.doesNotMatch(script, /ingest\.reject_reason/);
});
