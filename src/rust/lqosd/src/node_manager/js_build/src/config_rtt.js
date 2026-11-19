import { saveConfig, loadConfig, renderConfigMenu } from "./config/config_helper";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

const DEFAULTS = { green_ms: 0, yellow_ms: 100, red_ms: 200 };

function toInt(value, fallback) {
    const n = Number(value);
    if (!Number.isFinite(n)) return fallback;
    return Math.max(0, Math.round(n));
}

function getFormValues() {
    return {
        green_ms: toInt(document.getElementById("rttGreenMs").value, DEFAULTS.green_ms),
        yellow_ms: toInt(document.getElementById("rttYellowMs").value, DEFAULTS.yellow_ms),
        red_ms: toInt(document.getElementById("rttRedMs").value, DEFAULTS.red_ms),
    };
}

function setFormValues(values) {
    document.getElementById("rttGreenMs").value = values.green_ms;
    document.getElementById("rttYellowMs").value = values.yellow_ms;
    document.getElementById("rttRedMs").value = values.red_ms;
}

function thresholdsFromConfig(config) {
    const thresholds = config?.rtt_thresholds;
    if (!thresholds) return null;
    const green = toInt(thresholds.green_ms, DEFAULTS.green_ms);
    const yellow = toInt(thresholds.yellow_ms, DEFAULTS.yellow_ms);
    const red = toInt(thresholds.red_ms, DEFAULTS.red_ms);
    return { green_ms: green, yellow_ms: yellow, red_ms: red };
}

function validateThresholds(thresholds) {
    if (!thresholds) return { ok: true, message: "" };
    if (thresholds.red_ms <= 0) return { ok: false, message: t("config.rtt.validation.red_positive") };
    if (thresholds.green_ms > thresholds.yellow_ms) {
        return { ok: false, message: t("config.rtt.validation.green_le_yellow") };
    }
    if (thresholds.yellow_ms > thresholds.red_ms) {
        return { ok: false, message: t("config.rtt.validation.yellow_le_red") };
    }
    return { ok: true, message: "" };
}

function setInputsEnabled(enabled) {
    document.getElementById("rttGreenMs").disabled = !enabled;
    document.getElementById("rttYellowMs").disabled = !enabled;
    document.getElementById("rttRedMs").disabled = !enabled;
}

function renderPreview(thresholds) {
    const holder = document.getElementById("rttPreview");
    if (!holder) return;

    // The label is catalog text; the value keeps its literal `ms` unit.
    const swatch = (style, label, value) =>
        `<div><span class="badge ${style} me-1">${label}</span><code>${value}ms</code></div>`;

    holder.innerHTML = `
        <div class="d-flex align-items-center gap-3 flex-wrap">
            ${swatch("bg-success", t("config.rtt.preview.green"), thresholds.green_ms)}
            ${swatch("bg-warning text-dark", t("config.rtt.preview.yellow"), thresholds.yellow_ms)}
            ${swatch("bg-danger", t("config.rtt.preview.red"), thresholds.red_ms)}
        </div>
    `;
}

function setValidationMessage(ok, message) {
    const el = document.getElementById("rttValidationMessage");
    if (!el) return;
    if (!message) {
        el.textContent = "";
        el.className = "small mt-2";
        return;
    }
    el.textContent = message;
    el.className = `small mt-2 ${ok ? "text-muted" : "text-danger"}`;
}

function updateUiState() {
    const useDefaults = document.getElementById("useDefaultRttThresholds").checked;
    if (useDefaults) {
        setInputsEnabled(false);
        setFormValues(DEFAULTS);
        renderPreview(DEFAULTS);
        setValidationMessage(true, "");
        return;
    }

    setInputsEnabled(true);
    const thresholds = getFormValues();
    renderPreview(thresholds);
    const v = validateThresholds(thresholds);
    setValidationMessage(v.ok, v.message);
}

function updateConfigFromForm() {
    const useDefaults = document.getElementById("useDefaultRttThresholds").checked;
    if (useDefaults) {
        window.config.rtt_thresholds = null;
        return { ok: true };
    }

    const thresholds = getFormValues();
    const v = validateThresholds(thresholds);
    if (!v.ok) return { ok: false, message: v.message };
    window.config.rtt_thresholds = thresholds;
    return { ok: true };
}

renderConfigMenu("rtt");
// The note wraps `not` in `<strong>`, which the DOM applier cannot keep: the
// block is empty in the page and rendered here.
renderMarkupText({rttThresholdsNote: tHtml("config.rtt.note")});

loadConfig(() => {
    if (!window.config) return;

    const configured = thresholdsFromConfig(window.config);
    const useDefaults = configured === null;
    document.getElementById("useDefaultRttThresholds").checked = useDefaults;
    setFormValues(configured || DEFAULTS);

    document
        .getElementById("useDefaultRttThresholds")
        .addEventListener("change", updateUiState);
    ["rttGreenMs", "rttYellowMs", "rttRedMs"].forEach((id) => {
        document.getElementById(id).addEventListener("input", updateUiState);
    });

    document.getElementById("saveButton").addEventListener("click", () => {
        const res = updateConfigFromForm();
        if (!res.ok) {
            alert(res.message || t("config.rtt.validation.invalid"));
            return;
        }
        saveConfig(() => {
            alert(t("config.rtt.saved"));
        });
    });

    updateUiState();
});

