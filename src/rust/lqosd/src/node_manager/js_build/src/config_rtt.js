import { saveConfig, loadConfig, renderConfigMenu } from "./config/config_helper";

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
    const t = config?.rtt_thresholds;
    if (!t) return null;
    const green = toInt(t.green_ms, DEFAULTS.green_ms);
    const yellow = toInt(t.yellow_ms, DEFAULTS.yellow_ms);
    const red = toInt(t.red_ms, DEFAULTS.red_ms);
    return { green_ms: green, yellow_ms: yellow, red_ms: red };
}

function validateThresholds(t) {
    if (!t) return { ok: true, message: "" };
    if (t.red_ms <= 0) return { ok: false, message: "Red point must be > 0." };
    if (t.green_ms > t.yellow_ms) return { ok: false, message: "Green must be <= Yellow." };
    if (t.yellow_ms > t.red_ms) return { ok: false, message: "Yellow must be <= Red." };
    return { ok: true, message: "" };
}

function setInputsEnabled(enabled) {
    document.getElementById("rttGreenMs").disabled = !enabled;
    document.getElementById("rttYellowMs").disabled = !enabled;
    document.getElementById("rttRedMs").disabled = !enabled;
}

function renderPreview(t) {
    const holder = document.getElementById("rttPreview");
    if (!holder) return;

    const green = t.green_ms;
    const yellow = t.yellow_ms;
    const red = t.red_ms;

    holder.innerHTML = `
        <div class="d-flex align-items-center gap-3 flex-wrap">
            <div><span class="badge bg-success me-1">Green</span><code>${green}ms</code></div>
            <div><span class="badge bg-warning text-dark me-1">Yellow</span><code>${yellow}ms</code></div>
            <div><span class="badge bg-danger me-1">Red</span><code>${red}ms</code></div>
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
    const t = getFormValues();
    renderPreview(t);
    const v = validateThresholds(t);
    setValidationMessage(v.ok, v.message);
}

function updateConfigFromForm() {
    const useDefaults = document.getElementById("useDefaultRttThresholds").checked;
    if (useDefaults) {
        window.config.rtt_thresholds = null;
        return { ok: true };
    }

    const t = getFormValues();
    const v = validateThresholds(t);
    if (!v.ok) return { ok: false, message: v.message };
    window.config.rtt_thresholds = t;
    return { ok: true };
}

renderConfigMenu("rtt");

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
            alert(res.message || "Invalid RTT thresholds");
            return;
        }
        saveConfig(() => {
            alert("Configuration saved successfully!");
        });
    });

    updateUiState();
});

