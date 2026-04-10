import assert from "node:assert/strict";
import test from "node:test";

import {
    parseIpInput,
    parseIpv4Address,
    parseIpv6Address,
} from "./shaped_device_wire.mjs";

test("encodes IPv4 hosts and prefixes into byte tuples", () => {
    assert.deepEqual(parseIpv4Address("192.168.1.2"), [192, 168, 1, 2]);
    assert.deepEqual(parseIpInput("192.168.1.2,10.0.0.0/24", 4), [
        [[192, 168, 1, 2], 32],
        [[10, 0, 0, 0], 24],
    ]);
});

test("encodes IPv6 hosts into 16-byte tuples", () => {
    assert.deepEqual(parseIpv6Address("2001:db8::1"), [
        0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    ]);
    assert.deepEqual(parseIpInput("2001:db8::1/64", 6), [[
        [0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
        64,
    ]]);
});

test("encodes IPv4-mapped IPv6 addresses into 16-byte tuples", () => {
    assert.deepEqual(parseIpv6Address("::ffff:192.0.2.1"), [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xff, 0xff, 0xc0, 0x00, 0x02, 0x01,
    ]);
});

test("preserves invalid input as strings so UI validation can reject it", () => {
    assert.equal(parseIpv4Address("300.1.1.1"), null);
    assert.equal(parseIpv6Address("2001:::1"), null);
    assert.deepEqual(parseIpInput("300.1.1.1\n2001:::1", 4), [
        ["300.1.1.1", 32],
        ["2001:::1", 32],
    ]);
});
