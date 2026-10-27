import assert from "node:assert/strict";
import test from "node:test";

import {
    actionsEnabledForHealth,
    describeOutcome,
    isAdminRole,
    isSuspendedOverride,
    outcomeForGeneration,
    setActionPending,
    setActionSettled,
    validateRateInputs,
} from "./circuit_admin_actions.mjs";

class FakeClassList {
    constructor() {
        this.values = [];
    }

    add(...classes) {
        this.values.push(...classes);
    }

    contains(className) {
        return this.values.includes(className);
    }
}

class FakeElement {
    constructor(tagName) {
        this.tagName = tagName.toUpperCase();
        this.children = [];
        this.classList = new FakeClassList();
        this.disabled = false;
    }

    appendChild(child) {
        this.children.push(child);
        if (child.classList && child.classList.values.includes("lqos-action-lifecycle")) {
            this.lifecycle = child;
        }
        return child;
    }

    querySelectorAll(selector) {
        if (selector === "button") {
            return this.buttons ?? [];
        }
        return [];
    }

    querySelector(selector) {
        if (selector === ".lqos-action-lifecycle") {
            return this.lifecycle ?? null;
        }
        return null;
    }
}

function installFakeDocument() {
    globalThis.document = {
        createElement: (tagName) => new FakeElement(tagName),
        createTextNode: (textValue) => ({ textContent: String(textValue) }),
    };
}

test("role gating admits only Admin", () => {
    assert.equal(isAdminRole("Admin"), true);
    assert.equal(isAdminRole("ReadOnly"), false);
    assert.equal(isAdminRole("Anonymous"), false);
    assert.equal(isAdminRole(undefined), false);
    assert.equal(isAdminRole(""), false);
});

test("health gate disables actions when the bridge is down or unknown", () => {
    assert.equal(actionsEnabledForHealth("healthy"), true);
    assert.equal(actionsEnabledForHealth("degraded"), true);
    assert.equal(actionsEnabledForHealth("starting"), true);
    assert.equal(actionsEnabledForHealth("down"), false);
    assert.equal(actionsEnabledForHealth("pending"), false);
    assert.equal(actionsEnabledForHealth(undefined), false);
    assert.equal(actionsEnabledForHealth(null), false);
});

test("rate validation enforces the never-raw-zero rule", () => {
    const good = { download_min_mbps: 10, upload_min_mbps: 5, download_max_mbps: 100, upload_max_mbps: 50 };
    assert.deepEqual(validateRateInputs(good), []);

    // Zero rates are NOT operator edits — they are the suspend
    // encoding and the CSV cannot re-parse them.
    const zeroed = { ...good, download_min_mbps: 0 };
    assert.ok(validateRateInputs(zeroed).some((message) => message.includes("greater than 0")));

    const negative = { ...good, upload_max_mbps: -1 };
    assert.ok(validateRateInputs(negative).length > 0);

    const notANumber = { ...good, download_max_mbps: "abc" };
    assert.ok(validateRateInputs(notANumber).length > 0);

    const inverted = { ...good, download_min_mbps: 200 };
    assert.ok(
        validateRateInputs(inverted).some((message) => message.includes("must not exceed")),
    );
});

test("F-NA: the minted-generation poll sees a RETAINED generation (return-to-base)", () => {
    // The false-NO_ACK repro: revert-to-plan lands on a generation the
    // broker's ring ALREADY retains. Correlation by exact generation
    // (the minted key from the ws response) must find it — key-diff
    // never could.
    const minted = "g-base";
    const before = [{ generation: "g-override", state: "applied" }];
    assert.equal(outcomeForGeneration(before, minted), null,
        "the publisher has not reached the minted generation yet — keep polling");

    const published = [
        { generation: minted, state: "applied", updated_unix: 200 },
        { generation: "g-override", state: "applied", updated_unix: 100 },
    ];
    assert.deepEqual(outcomeForGeneration(published, minted), { status: "applied" },
        "revert-to-base settles APPLIED even though the key was already retained");

    const pendingFirst = [
        { generation: minted, state: { pending: { seq_pos: 1, seq_len: 1 } }, updated_unix: 200 },
        { generation: minted, state: "applied", updated_unix: 90 },
    ];
    assert.deepEqual(outcomeForGeneration(pendingFirst, minted), {
        status: "pending",
        seqPos: 1,
        seqLen: 1,
    }, "the NEWEST record wins for a retained generation (pending before its own applied)");
});

test("outcome mapping covers every broker record shape", () => {
    const records = [
        { generation: "g-pending", state: { pending: { seq_pos: 2, seq_len: 3 } } },
        { generation: "g-retry", state: { retrying: { attempt: 1, max: 3 } } },
        { generation: "g-applied", state: "applied" },
        { generation: "g-rejected", state: { rejected: { reason: "REJECTED_OBSERVE_MODE" } } },
    ];

    assert.deepEqual(outcomeForGeneration(records, "g-pending"), {
        status: "pending",
        seqPos: 2,
        seqLen: 3,
    });
    assert.deepEqual(outcomeForGeneration(records, "g-retry"), {
        status: "pending",
        retrying: true,
        seqPos: 1,
        seqLen: 3,
    });
    assert.deepEqual(outcomeForGeneration(records, "g-applied"), { status: "applied" });
    assert.deepEqual(outcomeForGeneration(records, "g-rejected"), {
        status: "rejected",
        reason: "REJECTED_OBSERVE_MODE",
    });
    assert.equal(outcomeForGeneration(records, "g-unknown"), null);
    assert.equal(outcomeForGeneration(null, "g-applied"), null);
});

test("outcome text is honest: typed reason verbatim, sequence progress visible", () => {
    assert.equal(describeOutcome({ status: "pending", seqPos: 2, seqLen: 3 }), "Applying to bridge (2/3)…");
    assert.equal(describeOutcome({ status: "pending", retrying: true, seqPos: 1, seqLen: 3 }), "Bridge retrying (attempt 1/3)…");
    assert.equal(describeOutcome({ status: "applied" }), "✓ Applied by bridge");
    assert.equal(describeOutcome({ status: "rejected", reason: "REJECTED_OBSERVE_MODE" }), "✗ REJECTED_OBSERVE_MODE");
    assert.equal(describeOutcome({ status: "noop" }), "Already in this state");
});

test("lifecycle DOM: pending disables the bar and shows the spinner text", () => {
    installFakeDocument();
    const root = new FakeElement("div");
    const button = new FakeElement("button");
    root.buttons = [button];

    setActionPending(root, "Applying to bridge (1/2)…");

    assert.equal(button.disabled, true);
    const node = root.lifecycle;
    assert.ok(node, "lifecycle line present");
    assert.equal(node.textContent, "Applying to bridge (1/2)…");
    assert.equal(node.classList.values.includes("text-secondary"), true);
});

test("lifecycle DOM: applied settles green and re-enables", () => {
    installFakeDocument();
    const root = new FakeElement("div");
    const button = new FakeElement("button");
    button.disabled = true;
    root.buttons = [button];

    const verdict = setActionSettled(root, { status: "applied" });

    assert.equal(verdict, "applied");
    assert.equal(button.disabled, false);
    assert.equal(root.lifecycle.textContent, "✓ Applied by bridge");
    assert.equal(root.lifecycle.classList.values.includes("text-success"), true);
});

test("lifecycle DOM: rejection surfaces the typed reason, never a retry", () => {
    installFakeDocument();
    const root = new FakeElement("div");
    const button = new FakeElement("button");
    button.disabled = true;
    root.buttons = [button];

    const verdict = setActionSettled(root, { status: "rejected", reason: "REJECTED_OBSERVE_MODE" });

    assert.equal(verdict, "rejected");
    assert.equal(button.disabled, false);
    assert.equal(root.lifecycle.textContent, "✗ REJECTED_OBSERVE_MODE");
    assert.equal(root.lifecycle.classList.values.includes("text-danger"), true);
});

test("lifecycle DOM: pending settle keeps controls disabled for the next poll", () => {
    installFakeDocument();
    const root = new FakeElement("div");
    const button = new FakeElement("button");
    root.buttons = [button];

    const verdict = setActionSettled(root, { status: "pending", seqPos: 1, seqLen: 2 });

    assert.equal(verdict, "pending");
    assert.equal(button.disabled, true, "still applying — bar stays disabled");
});

test("isSuspendedOverride recognizes only the full 0.01 floor", () => {
    assert.equal(isSuspendedOverride(null), false);
    assert.equal(isSuspendedOverride(undefined), false);
    assert.equal(
        isSuspendedOverride({
            min_download_bandwidth: 0.01,
            max_download_bandwidth: 0.01,
            min_upload_bandwidth: 0.01,
            max_upload_bandwidth: 0.01,
        }),
        true,
    );
    assert.equal(
        isSuspendedOverride({
            min_download_bandwidth: 0.01,
            max_download_bandwidth: 0.01,
            min_upload_bandwidth: 0.01,
            max_upload_bandwidth: 0.02,
        }),
        false,
        "a partial floor is a rate override, not a suspend",
    );
    assert.equal(
        isSuspendedOverride({
            min_download_bandwidth: 0.01,
            max_download_bandwidth: 0.01,
            min_upload_bandwidth: null,
            max_upload_bandwidth: null,
        }),
        false,
        "missing directions are not a suspend",
    );
});

test("lifecycle DOM: a no-op dispatch settles immediately, never waits on a generation", () => {
    installFakeDocument();
    const root = new FakeElement("div");
    const button = new FakeElement("button");
    button.disabled = true;
    root.buttons = [button];

    const verdict = setActionSettled(root, { status: "noop" });

    assert.equal(verdict, "noop");
    assert.equal(button.disabled, false, "no bridge work exists — bar re-enables");
    assert.equal(root.lifecycle.textContent, "Already in this state");
    assert.equal(root.lifecycle.classList.values.includes("text-secondary"), true);
    assert.equal(root.lifecycle.classList.values.includes("text-danger"), false,
        "a no-op is not a failure");
});
