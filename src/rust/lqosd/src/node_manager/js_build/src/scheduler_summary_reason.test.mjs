import assert from "node:assert/strict";
import test from "node:test";

import {clearCatalogs, EN, ES, installCatalogs, installKeyEchoCatalogs} from "../i18n/test_catalogs.mjs";
import {schedulerDegradedReason} from "./scheduler_summary_reason.mjs";

const NAME = "UISP";
const IMPORT_LINE = "UISP import failing";
const TRUNK_DETAIL = EN.messages["scheduler.degraded.detail.bootstrap_trunk"];
const PASS_DETAIL = EN.messages["scheduler.degraded.detail.pass_through"];

test.beforeEach(() => installCatalogs());
test.afterEach(clearCatalogs);

test("import_failing names the import axis (unchanged wording)", () => {
    const reason = schedulerDegradedReason("import_failing", NAME, IMPORT_LINE);
    assert.match(reason.title, /UISP import failing/);
    assert.equal(reason.subtitle, IMPORT_LINE);
});

test("boot-world-only reasons NEVER say 'import failing'", () => {
    for (const served of ["boot_world_bootstrap_trunk", "boot_world_pass_through"]) {
        const reason = schedulerDegradedReason(served, NAME, IMPORT_LINE);
        assert.doesNotMatch(reason.title, /import failing/i, served);
        assert.doesNotMatch(reason.subtitle, /import failing/i, served);
    }
});

test("bootstrap trunk names the boot tier + the boot-time detail", () => {
    const reason = schedulerDegradedReason("boot_world_bootstrap_trunk", NAME, IMPORT_LINE);
    assert.match(reason.title, /bootstrap trunk/);
    assert.match(reason.title, /at boot/);
    assert.equal(reason.subtitle, TRUNK_DETAIL);
});

test("pass-through names the pass-through posture", () => {
    const reason = schedulerDegradedReason("boot_world_pass_through", NAME, IMPORT_LINE);
    assert.match(reason.title, /pass-through/);
    assert.equal(reason.subtitle, PASS_DETAIL);
});

test("combined reasons render BOTH axes with the boot world FIRST", () => {
    const trunk = schedulerDegradedReason(
        "import_failing_and_boot_world_bootstrap_trunk",
        NAME,
        IMPORT_LINE,
    );
    assert.match(trunk.title, /bootstrap trunk/);
    assert.ok(trunk.subtitle.startsWith(TRUNK_DETAIL), "boot world must come first");
    assert.match(trunk.subtitle, /UISP import failing/, "the import axis must not be dropped");

    const pass = schedulerDegradedReason(
        "import_failing_and_boot_world_pass_through",
        NAME,
        IMPORT_LINE,
    );
    assert.match(pass.title, /pass-through/);
    assert.ok(pass.subtitle.startsWith(PASS_DETAIL), "boot world must come first");
    assert.match(pass.subtitle, /UISP import failing/, "the import axis must not be dropped");
});

test("absent/unknown reason never claims the import axis", () => {
    for (const served of [null, undefined, "", "something_new"]) {
        const reason = schedulerDegradedReason(served, NAME, IMPORT_LINE);
        assert.doesNotMatch(reason.title, /import failing/i);
        assert.doesNotMatch(reason.subtitle, /import failing/i);
    }
});

test("the served reason is a code: every branch maps to catalog keys both locales define", () => {
    // With no catalog installed every render returns the KEY it asked for, so the
    // code-to-key mapping is observable without a second copy of it in the test.
    installKeyEchoCatalogs();
    const served = [
        "import_failing",
        "boot_world_bootstrap_trunk",
        "boot_world_pass_through",
        "import_failing_and_boot_world_bootstrap_trunk",
        "import_failing_and_boot_world_pass_through",
        null,
        "something_new",
    ];
    for (const reason of served) {
        const rendered = schedulerDegradedReason(reason, NAME, "");
        for (const text of [rendered.title, rendered.subtitle]) {
            assert.match(text, /^scheduler\./, `${reason}: expected a catalog key, got "${text}"`);
            assert.ok(EN.messages[text], `en.json is missing "${text}"`);
            assert.ok(ES.messages[text], `es.json is missing "${text}"`);
        }
    }
});

test("the degraded reason follows the served locale", () => {
    installCatalogs(ES);
    const trunk = schedulerDegradedReason("boot_world_bootstrap_trunk", NAME, IMPORT_LINE);
    assert.equal(trunk.title, ES.messages["scheduler.degraded.title.bootstrap_trunk"]);
    assert.equal(trunk.subtitle, ES.messages["scheduler.degraded.detail.bootstrap_trunk"]);
    assert.match(trunk.title, /troncal de arranque/);

    const combined = schedulerDegradedReason(
        "import_failing_and_boot_world_pass_through",
        NAME,
        IMPORT_LINE,
    );
    assert.equal(combined.subtitle, `${ES.messages["scheduler.degraded.detail.pass_through"]} · ${IMPORT_LINE}`);
});
