import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

// Structural contract for the DRIVER's 4 v3 UI change requests
// (docs/v3_ui_change_requests_work_order.md). Guards the changes that are
// markup/CSS (not unit-testable logic) so a later refactor cannot silently
// regress them.
const srcDir = new URL(".", import.meta.url);
const static2 = new URL("../../static2/", srcDir);
const read = (name) => readFile(fileURLToPath(new URL(name, srcDir)), "utf8");
const readStatic = (name) => readFile(fileURLToPath(new URL(name, static2)), "utf8");

test("(1) the Reload LibreQoS sidebar item exists + is admin-gated", async () => {
    const [page, source] = await Promise.all([readStatic("template.html"), read("template.js")]);
    assert.match(page, /id="reloadLibreQoSItem"/);
    assert.match(page, /id="reloadLibreQoSLink"/);
    // Admin gate: hidden unless isAdminRole, and it drives the existing handler.
    assert.match(source, /import \{isAdminRole\} from "\.\/circuit_admin_actions\.mjs";/);
    assert.match(source, /function initReloadLibreQoS\(\)/);
    assert.match(source, /if \(!isAdminRole\(window\.USER_ROLE\)\)/);
    assert.match(source, /triggerReloadLibreQoS\("Re-syncing integration and applying/);
    assert.match(source, /^\s*initReloadLibreQoS\(\);/m);
});

test("(2) the client-rendered status buttons carry no p-0 (padding parity)", async () => {
    const source = await read("template.js");
    const buttonClasses = [...source.matchAll(/class="nav-link btn btn-link[^"]*"/g)].map((m) => m[0]);
    assert.ok(buttonClasses.length >= 2, "scheduler + urgent buttons present");
    for (const cls of buttonClasses) {
        assert.ok(!cls.includes("p-0"), `status button must not zero its padding: ${cls}`);
    }
});

test("(3) the sidebar nav-link vertical padding is tightened", async () => {
    const css = await readStatic("node_manager.css");
    assert.match(css, /\.sidebar \.nav-link \{[^}]*padding: 0\.4rem 0\.65rem;/s);
});

test("(4) circuit.html puts Edit Rates in the Packet Capture row", async () => {
    const page = await readStatic("circuit.html");
    const cap = page.indexOf('id="captureButton"');
    const admin = page.indexOf('id="circuitAdminActions"');
    assert.ok(cap !== -1 && admin !== -1 && cap < admin, "both targets present, capture first");
    assert.ok(!page.slice(cap, admin).includes("</tr>"), "capture + admin actions share one row");
    assert.match(page, /id="circuitAdminActions" class="text-end/);
});

test("(5) the Bridge sidebar item links to bridge.html (client re-render keeps the href)", async () => {
    // L4 §5b regression: the server template emits href="bridge.html", but the
    // client `renderBridgeHealthStatus` re-renders the item every health poll —
    // if it emits href="#" the button stops navigating.
    const source = await read("template.js");
    const start = source.indexOf("function renderBridgeHealthStatus");
    const end = source.indexOf("function scheduleNextBridgeHealthPoll");
    assert.ok(start !== -1 && end > start, "renderBridgeHealthStatus present");
    const body = source.slice(start, end);
    assert.match(body, /href="bridge\.html"/, "the client re-render must keep the Bridge page link");
    assert.ok(!/href="#"/.test(body), "the client re-render must NOT override the link with '#'");
});
