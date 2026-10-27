import assert from "node:assert/strict";
import test from "node:test";

import {BOOTSTRAP_TRUNK_DETAIL, PASS_THROUGH_DETAIL, schedulerDegradedReason} from "./scheduler_summary_reason.mjs";

const NAME = "UISP";
const IMPORT_LINE = "UISP import failing";

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
    assert.equal(reason.subtitle, BOOTSTRAP_TRUNK_DETAIL);
});

test("pass-through names the pass-through posture", () => {
    const reason = schedulerDegradedReason("boot_world_pass_through", NAME, IMPORT_LINE);
    assert.match(reason.title, /pass-through/);
    assert.equal(reason.subtitle, PASS_THROUGH_DETAIL);
});

test("combined reasons render BOTH axes with the boot world FIRST", () => {
    const trunk = schedulerDegradedReason(
        "import_failing_and_boot_world_bootstrap_trunk",
        NAME,
        IMPORT_LINE,
    );
    assert.match(trunk.title, /bootstrap trunk/);
    assert.ok(trunk.subtitle.startsWith(BOOTSTRAP_TRUNK_DETAIL), "boot world must come first");
    assert.match(trunk.subtitle, /UISP import failing/, "the import axis must not be dropped");

    const pass = schedulerDegradedReason(
        "import_failing_and_boot_world_pass_through",
        NAME,
        IMPORT_LINE,
    );
    assert.match(pass.title, /pass-through/);
    assert.ok(pass.subtitle.startsWith(PASS_THROUGH_DETAIL), "boot world must come first");
    assert.match(pass.subtitle, /UISP import failing/, "the import axis must not be dropped");
});

test("absent/unknown reason never claims the import axis", () => {
    for (const served of [null, undefined, "", "something_new"]) {
        const reason = schedulerDegradedReason(served, NAME, IMPORT_LINE);
        assert.doesNotMatch(reason.title, /import failing/i);
        assert.doesNotMatch(reason.subtitle, /import failing/i);
    }
});
