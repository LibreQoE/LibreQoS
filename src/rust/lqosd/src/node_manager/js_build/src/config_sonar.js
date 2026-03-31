import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function arrayToString(arr) {
    return arr ? arr.join(', ') : '';
}

function stringToArray(str) {
    return str ? str.split(',').map(s => s.trim()).filter(s => s.length > 0) : [];
}

function sanitizeRecurringRate(rule = {}) {
    return {
        enabled: rule.enabled !== false,
        service_name: (rule.service_name ?? "").trim(),
        download_mbps: Number(rule.download_mbps ?? 0),
        upload_mbps: Number(rule.upload_mbps ?? 0),
    };
}

function buildRecurringRateRow(rule = {}) {
    const row = document.createElement("tr");
    const sanitized = sanitizeRecurringRate(rule);
    row.innerHTML = `
        <td><input type="checkbox" class="form-check-input recurring-enabled"></td>
        <td><input type="text" class="form-control form-control-sm recurring-service-name"></td>
        <td><input type="number" min="0" step="0.01" class="form-control form-control-sm recurring-download"></td>
        <td><input type="number" min="0" step="0.01" class="form-control form-control-sm recurring-upload"></td>
        <td class="text-end"><button type="button" class="btn btn-outline-danger btn-sm recurring-remove"><i class="fa fa-trash"></i></button></td>
    `;
    row.querySelector(".recurring-enabled").checked = sanitized.enabled;
    row.querySelector(".recurring-service-name").value = sanitized.service_name;
    row.querySelector(".recurring-download").value = Number.isFinite(sanitized.download_mbps) ? sanitized.download_mbps : 0;
    row.querySelector(".recurring-upload").value = Number.isFinite(sanitized.upload_mbps) ? sanitized.upload_mbps : 0;
    row.querySelector(".recurring-remove").addEventListener("click", () => row.remove());
    return row;
}

function renderRecurringRateRows(rules = []) {
    const tbody = document.getElementById("recurringServiceRatesBody");
    tbody.innerHTML = "";
    rules.forEach((rule) => tbody.appendChild(buildRecurringRateRow(rule)));
}

function collectRecurringRateRows() {
    return Array.from(document.querySelectorAll("#recurringServiceRatesBody tr"))
        .map((row) => ({
            enabled: row.querySelector(".recurring-enabled").checked,
            service_name: row.querySelector(".recurring-service-name").value.trim(),
            download_mbps: Number(row.querySelector(".recurring-download").value),
            upload_mbps: Number(row.querySelector(".recurring-upload").value),
        }))
        .filter((rule) => rule.service_name.length > 0);
}

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableSonar").checked) {
        const apiUrl = document.getElementById("sonarApiUrl").value.trim();
        if (!apiUrl) {
            alert("API URL is required when Sonar integration is enabled");
            return false;
        }
        try {
            new URL(apiUrl);
        } catch {
            alert("API URL must be a valid URL");
            return false;
        }

        const apiKey = document.getElementById("sonarApiKey").value.trim();
        if (!apiKey) {
            alert("API Key is required when Sonar integration is enabled");
            return false;
        }

        const snmpCommunity = document.getElementById("snmpCommunity").value.trim();
        if (!snmpCommunity) {
            alert("SNMP Community is required when Sonar integration is enabled");
            return false;
        }
    }

    for (const rule of collectRecurringRateRows()) {
        if (!Number.isFinite(rule.download_mbps) || rule.download_mbps <= 0) {
            alert(`Recurring service "${rule.service_name}" must have a valid download rate`);
            return false;
        }
        if (!Number.isFinite(rule.upload_mbps) || rule.upload_mbps <= 0) {
            alert(`Recurring service "${rule.service_name}" must have a valid upload rate`);
            return false;
        }
    }
    return true;
}

function updateConfig() {
    // Update only the sonar_integration section
    window.config.sonar_integration = {
        enable_sonar: document.getElementById("enableSonar").checked,
        sonar_api_url: document.getElementById("sonarApiUrl").value.trim(),
        sonar_api_key: document.getElementById("sonarApiKey").value.trim(),
        snmp_community: document.getElementById("snmpCommunity").value.trim(),
        airmax_model_ids: stringToArray(document.getElementById("airmaxModelIds").value),
        ltu_model_ids: stringToArray(document.getElementById("ltuModelIds").value),
        active_status_ids: stringToArray(document.getElementById("activeStatusIds").value),
        recurring_excluded_service_names: stringToArray(document.getElementById("recurringExcludedServiceNames").value),
        recurring_service_rates: collectRecurringRateRows(),
    };
}

// Render the configuration menu
renderConfigMenu('sonar');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.sonar_integration) {
        const sonar = window.config.sonar_integration;
        
        // Boolean field
        document.getElementById("enableSonar").checked = 
            sonar.enable_sonar ?? false;

        // String fields
        document.getElementById("sonarApiUrl").value = 
            sonar.sonar_api_url ?? "";
        document.getElementById("sonarApiKey").value = 
            sonar.sonar_api_key ?? "";
        document.getElementById("snmpCommunity").value = 
            sonar.snmp_community ?? "public";

        // Array fields (convert to comma-separated strings)
        document.getElementById("airmaxModelIds").value = 
            arrayToString(sonar.airmax_model_ids);
        document.getElementById("ltuModelIds").value = 
            arrayToString(sonar.ltu_model_ids);
        document.getElementById("activeStatusIds").value = 
            arrayToString(sonar.active_status_ids);
        document.getElementById("recurringExcludedServiceNames").value =
            arrayToString(sonar.recurring_excluded_service_names);
        renderRecurringRateRows(sonar.recurring_service_rates ?? []);
        document.getElementById("addRecurringRateRow").addEventListener("click", () => {
            document.getElementById("recurringServiceRatesBody").appendChild(buildRecurringRateRow());
        });

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
        console.error("Sonar integration configuration not found in window.config");
    }
});
