import assert from "node:assert/strict";
import test from "node:test";

import {directHttpUrlFromLocation, sslDisableRedirectTarget} from "./ssl_redirect.mjs";

test("builds direct HTTP URL from HTTPS management IP", () => {
    assert.equal(
        directHttpUrlFromLocation({ href: "https://192.168.122.225/config_ssl?tab=ssl" }),
        "http://192.168.122.225:9123/",
    );
});

test("builds direct HTTP URL from HTTPS hostname", () => {
    assert.equal(
        directHttpUrlFromLocation({ href: "https://libreqos.example.com/config_ssl" }),
        "http://libreqos.example.com:9123/",
    );
});

test("prefers server supplied SSL disable target URL", () => {
    assert.equal(
        sslDisableRedirectTarget(
            { target_url: "http://192.168.122.225:9123/" },
            { href: "https://192.168.122.225/config_ssl" },
        ),
        "http://192.168.122.225:9123/",
    );
});
