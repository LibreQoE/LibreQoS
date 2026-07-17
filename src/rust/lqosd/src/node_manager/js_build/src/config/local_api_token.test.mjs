import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
    abbreviateLocalApiKeyId,
    clearLocalApiKeyInput,
    copyLocalApiToken,
    formatLocalApiKeyCreatedAt,
    setLocalApiKeyCreationPending,
    validateLocalApiKeyName,
} from "./local_api_token.js";

test("name validation trims and rejects blank duplicate and long names", () => {
    assert.deepEqual(validateLocalApiKeyName("  Monitoring  "), { ok: true, name: "Monitoring" });
    assert.equal(validateLocalApiKeyName(" ").ok, false);
    assert.equal(validateLocalApiKeyName("monitoring", ["Monitoring"]).ok, false);
    assert.equal(validateLocalApiKeyName("x".repeat(65)).ok, false);
});

test("metadata formatters abbreviate IDs and handle invalid dates", () => {
    assert.equal(abbreviateLocalApiKeyId("12345678-1234"), "12345678…");
    assert.equal(formatLocalApiKeyCreatedAt(0), "Unknown");
    assert.notEqual(formatLocalApiKeyCreatedAt(1, "en-US"), "Unknown");
});

test("closing the dialog clears and hides the one-time secret", () => {
    const input = { value: "lqos_api_secret", type: "text" };
    clearLocalApiKeyInput(input);
    assert.deepEqual(input, { value: "", type: "password" });
});

test("the key dialog clears secrets on close", () => {
    const controller = readFileSync(new URL("../config_lts.js", import.meta.url), "utf8");
    assert.match(controller, /hidden\.bs\.modal", resetLocalApiKeyModal/);
    assert.match(controller, /timeoutMs: 15000/);
    assert.match(controller, /localApiKeysStatus/);
    assert.doesNotMatch(controller, /localStorage|sessionStorage/);
});

test("creation pending state disables every modal exit and reports progress", () => {
    const dismissButtons = [{ disabled: false }, { disabled: false }];
    const modal = { querySelectorAll() { return dismissButtons; } };
    const attributes = new Map();
    const confirm = {
        disabled: false,
        setAttribute(name, value) { attributes.set(name, value); },
        removeAttribute(name) { attributes.delete(name); },
    };
    const status = { className: "", textContent: "" };

    setLocalApiKeyCreationPending(confirm, modal, status, true);
    assert.equal(confirm.disabled, true);
    assert.equal(attributes.get("aria-busy"), "true");
    assert.deepEqual(dismissButtons.map((button) => button.disabled), [true, true]);
    assert.equal(status.textContent, "Generating the API key…");

    setLocalApiKeyCreationPending(confirm, modal, status, false);
    assert.equal(confirm.disabled, false);
    assert.equal(attributes.has("aria-busy"), false);
    assert.deepEqual(dismissButtons.map((button) => button.disabled), [false, false]);
    assert.equal(status.textContent, "");
});

test("copy uses the secure clipboard", async () => {
    let copied = "";
    const input = { value: " lqos_api_token ", type: "password" };
    await copyLocalApiToken(input, { writeText(value) { copied = value; } }, undefined, true);
    assert.equal(copied, "lqos_api_token");
    assert.equal(input.type, "password");
});

test("copy falls back to selection on direct HTTP", async () => {
    let selected = false;
    let focusRestored = false;
    const input = { value: "token", type: "password", focus() {}, select() { selected = true; } };
    await copyLocalApiToken(input, undefined, {
        activeElement: { focus() { focusRestored = true; } },
        execCommand(command) { return command === "copy"; },
    }, false);
    assert.equal(selected, true);
    assert.equal(focusRestored, true);
    assert.equal(input.type, "password");
});
