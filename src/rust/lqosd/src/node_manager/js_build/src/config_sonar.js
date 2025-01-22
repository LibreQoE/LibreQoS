import {loadConfig} from "./config/config_helper";

function arrayToString(arr) {
    return arr ? arr.join(', ') : '';
}

function stringToArray(str) {
    return str ? str.split(',').map(s => s.trim()).filter(s => s.length > 0) : [];
}

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
    } else {
        console.error("Sonar integration configuration not found in window.config");
    }
});
