import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.long_term_stats) {
        const lts = window.config.long_term_stats;
        
        // Boolean fields
        document.getElementById("gatherStats").checked = lts.gather_stats ?? true;
        document.getElementById("useInsight").checked = lts.use_insight ?? false;

        // Numeric fields
        if (lts.collation_period_seconds) {
            document.getElementById("collationPeriod").value = lts.collation_period_seconds;
        }
        if (lts.uisp_reporting_interval_seconds) {
            document.getElementById("uispInterval").value = lts.uisp_reporting_interval_seconds;
        }

        // Optional string fields
        document.getElementById("licenseKey").value = lts.license_key ?? "";
        document.getElementById("ltsUrl").value = lts.lts_url ?? "";
    } else {
        console.error("Long-term stats configuration not found in window.config");
    }
});
