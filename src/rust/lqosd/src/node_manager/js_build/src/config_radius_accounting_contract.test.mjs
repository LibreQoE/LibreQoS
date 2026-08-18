import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";

const sourceDirectory = new URL(".", import.meta.url);
const configSourcePath = fileURLToPath(new URL("config_radius_accounting.js", sourceDirectory));
const configPagePath = fileURLToPath(new URL("../../static2/config_radius_accounting.html", sourceDirectory));

test("RADIUS username matching toggle has a complete form contract", async () => {
    const [source, page] = await Promise.all([
        readFile(configSourcePath, "utf8"),
        readFile(configPagePath, "utf8"),
    ]);

    assert.match(page, /id="radiusMatchShapedDevicesByUsername"/);
    assert.match(source, /application\.match_shaped_devices_by_username = false/);
    assert.match(source, /radiusMatchShapedDevicesByUsername"\)\?\.checked/);
    assert.match(source, /radiusMatchShapedDevicesByUsername"\)\.checked = !!application\.match_shaped_devices_by_username/);
});
