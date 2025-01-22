import {saveConfig, loadConfig} from "./config/config_helper";

function validateConfig() {
    // Validate numeric fields
    const collationPeriod = parseInt(document.getElementById("collationPeriod").value);
    if (isNaN(collationPeriod) || collationPeriod < 1) {
        alert("Collation Period must be a number greater than 0");
        return false;
    }

    const uispInterval = parseInt(document.getElementById("uispInterval").value);
    if (isNaN(uispInterval) || uispInterval < 0) {
        alert("UISP Reporting Interval must be a number of at least 0");
        return false;
    }

    // Validate URL format if provided
    const ltsUrl = document.getElementById("ltsUrl").value.trim();
    if (ltsUrl) {
        try {
            new URL(ltsUrl);
        } catch {
            alert("LTS Server URL must be a valid URL");
            return false;
        }
    }

    return true;
}

function updateConfig() {
    // Update only the long-term stats section
    window.config.long_term_stats = {
        gather_stats: document.getElementById("gatherStats").checked,
        collation_period_seconds: parseInt(document.getElementById("collationPeriod").value),
        license_key: document.getElementById("licenseKey").value.trim() || null,
        uisp_reporting_interval_seconds: parseInt(document.getElementById("uispInterval").value) || null,
        lts_url: document.getElementById("ltsUrl").value.trim() || null,
        use_insight: document.getElementById("useInsight").checked
    };
}

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.long_term_stats) {
        const lts = window.config.long_term_stats;
        
        // Boolean fields
        document.getElementById("gatherStats").checked = lts.gather_stats ?? true;
        document.getElementById("useInsight").checked = lts.use_insight ?? false;

        // Numeric fields
        document.getElementById("collationPeriod").value = lts.collation_period_seconds ?? 60;
        document.getElementById("uispInterval").value = lts.uisp_reporting_interval_seconds ?? 300;

        // Optional string fields
        document.getElementById("licenseKey").value = lts.license_key ?? "";
        document.getElementById("ltsUrl").value = lts.lts_url ?? "";

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert("Configuration saved successfully!");
                });
            }
        });
    } else {
        console.error("Long-term stats configuration not found in window.config");
    }
});
