import assert from "node:assert/strict";
import {test} from "node:test";
import {insightTileUrls, resolveInsightBase} from "./site_map_config.mjs";

test("defaults to the Insight base when unset/blank/malformed", () => {
    for (const value of [undefined, null, "", "   ", "http://[bad", "not a url"]) {
        assert.equal(resolveInsightBase(value), "https://insight.libreqos.com/");
    }
});

test("normalizes a configured base to a trailing slash", () => {
    assert.equal(resolveInsightBase("https://tiles.example"), "https://tiles.example/");
    assert.equal(resolveInsightBase("tiles.example/"), "https://tiles.example/");
    assert.equal(resolveInsightBase("https://tiles.example/base"), "https://tiles.example/base/");
});

test("derives the bbox URL and the tile host/path", () => {
    const urls = insightTileUrls("https://tiles.example/base/");
    assert.equal(urls.base, "https://tiles.example/base/");
    assert.equal(urls.bboxUrl, "https://tiles.example/base/tiles/api/bbox");
    assert.equal(urls.tileHost, "tiles.example");
    assert.equal(urls.tilePath, "/base/tiles/{z}/{y}/{x}.png");
});

test("the default base yields the historical endpoints", () => {
    const urls = insightTileUrls(undefined);
    assert.equal(urls.bboxUrl, "https://insight.libreqos.com/tiles/api/bbox");
    assert.equal(urls.tileHost, "insight.libreqos.com");
    assert.equal(urls.tilePath, "/tiles/{z}/{y}/{x}.png");
});
