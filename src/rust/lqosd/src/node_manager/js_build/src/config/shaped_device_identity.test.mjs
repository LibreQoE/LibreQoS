import assert from "node:assert/strict";
import test from "node:test";

import {
    handleShapedDeviceActionClick,
    shapedDeviceIdFromElement,
    shapedDeviceIdMatches,
    shapedDeviceRowForId,
} from "./shaped_device_identity.mjs";

function actionEvent(deviceId) {
    return {
        prevented: false,
        currentTarget: {
            getAttribute(name) {
                return name === "data-device-id" ? deviceId : null;
            },
        },
        preventDefault() {
            this.prevented = true;
        },
    };
}

test("reads numeric data-device-id values as strings", () => {
    const element = {
        getAttribute(name) {
            return name === "data-device-id" ? "123" : null;
        },
    };

    assert.equal(shapedDeviceIdFromElement(element), "123");
});

test("preserves leading zeros in data-device-id values", () => {
    const element = {
        getAttribute(name) {
            return name === "data-device-id" ? "00123" : null;
        },
    };

    assert.equal(shapedDeviceIdFromElement(element), "00123");
});

test("matches shaped device rows by string-stable device id", () => {
    assert.equal(shapedDeviceIdMatches({ device_id: "00123" }, "00123"), true);
    assert.equal(shapedDeviceIdMatches({ device_id: 123 }, "123"), true);
    assert.equal(shapedDeviceIdMatches({ device_id: "123" }, "00123"), false);
});

test("finds shaped device rows by string-stable device id", () => {
    const rows = [
        { device_id: "123", device_name: "numeric" },
        { device_id: "00123", device_name: "leading zero" },
    ];

    assert.equal(shapedDeviceRowForId(rows, "123")?.device_name, "numeric");
    assert.equal(shapedDeviceRowForId(rows, 123)?.device_name, "numeric");
    assert.equal(shapedDeviceRowForId(rows, "00123")?.device_name, "leading zero");
    assert.equal(shapedDeviceRowForId(rows, "456"), undefined);
});

test("passes clicked numeric device ids to action callbacks as strings", () => {
    const event = actionEvent("123");
    let received = null;

    handleShapedDeviceActionClick(event, (deviceId) => {
        received = deviceId;
    });

    assert.equal(event.prevented, true);
    assert.equal(received, "123");
});

test("click action can select numeric and leading-zero rows", () => {
    const rows = [
        { device_id: "123", device_name: "numeric" },
        { device_id: "00123", device_name: "leading zero" },
    ];
    const selected = [];

    ["123", "00123"].forEach((deviceId) => {
        const event = actionEvent(deviceId);
        handleShapedDeviceActionClick(event, (clickedDeviceId) => {
            selected.push(shapedDeviceRowForId(rows, clickedDeviceId)?.device_name);
        });
        assert.equal(event.prevented, true);
    });

    assert.deepEqual(selected, ["numeric", "leading zero"]);
});

test("does not invoke action callbacks for empty device ids", () => {
    const event = actionEvent("");
    let called = false;

    handleShapedDeviceActionClick(event, () => {
        called = true;
    });

    assert.equal(event.prevented, true);
    assert.equal(called, false);
});
