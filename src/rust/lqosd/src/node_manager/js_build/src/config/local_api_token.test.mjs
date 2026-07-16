import assert from "node:assert/strict";
import test from "node:test";

import {
    copyLocalApiToken,
    generateLocalApiToken,
} from "./local_api_token.js";

test("generateLocalApiToken returns 32 random bytes as lowercase hex", () => {
    const cryptoApi = {
        getRandomValues(bytes) {
            bytes.forEach((_, index) => {
                bytes[index] = index;
            });
            return bytes;
        },
    };

    assert.equal(
        generateLocalApiToken(cryptoApi),
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    );
});

test("generateLocalApiToken refuses insecure fallback generation", () => {
    assert.throws(
        () => generateLocalApiToken({}),
        /Secure token generation is unavailable/,
    );
});

test("copyLocalApiToken uses the secure clipboard when available", async () => {
    let copied = "";
    const input = { value: " token-value ", type: "password" };
    await copyLocalApiToken(
        input,
        {
            writeText(value) {
                copied = value;
            },
        },
        undefined,
        true,
    );

    assert.equal(copied, "token-value");
    assert.equal(input.type, "password");
});

test("copyLocalApiToken falls back to selection on direct HTTP", async () => {
    let selected = false;
    let focusRestored = false;
    const input = {
        value: "token-value",
        type: "password",
        focus() {},
        select() {
            selected = true;
        },
    };
    await copyLocalApiToken(
        input,
        undefined,
        {
            activeElement: {
                focus() {
                    focusRestored = true;
                },
            },
            execCommand(command) {
                return command === "copy";
            },
        },
        false,
    );

    assert.equal(selected, true);
    assert.equal(focusRestored, true);
    assert.equal(input.type, "password");
});
