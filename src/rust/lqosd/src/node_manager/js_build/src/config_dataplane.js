import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";
import {readBandwidthMbps, applyBandwidthMbps} from "./config/dataplane_hygiene.mjs";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

// The v3 `[dataplane]` root-shaper bandwidth editor. `window.config.dataplane`
// is the single source (`download_mbps`/`upload_mbps`); the read falls back to
// the legacy `[queues]` keys, and the save MIRRORS into `[queues]` so a
// not-yet-migrated reader agrees during the transition. The save itself is
// generic (`saveConfig` round-trips `window.config` as TOML), so no new
// endpoint is needed; `DataplaneConfig` is serde + `deny_unknown_fields`.

// The engine accepts a DECIMAL Mbps (`12.5` => 12_500_000 bps), and so does
// the v3 config (`[dataplane] download_mbps`/`[queues] *_bandwidth_mbps` are
// decimals) ⇒ parse a float, never `parseInt` (which would truncate 12.5 -> 12).
function readField(id) {
    return parseFloat(document.getElementById(id).value);
}

function validateConfig() {
    const download = readField("dataplaneDownload");
    if (!Number.isFinite(download) || download <= 0) {
        alert(t("config.dataplane.validation.download"));
        return false;
    }
    const upload = readField("dataplaneUpload");
    if (!Number.isFinite(upload) || upload <= 0) {
        alert(t("config.dataplane.validation.upload"));
        return false;
    }
    return true;
}

function updateConfig() {
    applyBandwidthMbps(window.config, readField("dataplaneDownload"), readField("dataplaneUpload"));
}

renderConfigMenu('dataplane');
// The note names the `lqos.conf` sections in `<code>`, which the DOM applier
// cannot keep: the block is empty in the page and rendered here.
renderMarkupText({dataplaneSectionNote: tHtml("config.dataplane.note")});

loadConfig(() => {
    if (!window.config) {
        // translatable: false — "Configuration not found in window.config" is a console diagnostic, never rendered
        console.error("Configuration not found in window.config");
        return;
    }
    const {download, upload} = readBandwidthMbps(window.config);
    document.getElementById("dataplaneDownload").value = download;
    document.getElementById("dataplaneUpload").value = upload;
    // `step="any"` on the inputs lets a decimal be typed; `readField` keeps it.

    document.getElementById('saveButton').addEventListener('click', () => {
        if (validateConfig()) {
            updateConfig();
            saveConfig(() => {
                alert(t("config.dataplane.saved"));
            });
        }
    });
});
