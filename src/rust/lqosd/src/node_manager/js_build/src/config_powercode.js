import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.powercode_integration) {
        const powercode = window.config.powercode_integration;
        
        // Boolean field
        document.getElementById("enablePowercode").checked = 
            powercode.enable_powercode ?? false;

        // String fields
        document.getElementById("powercodeApiKey").value = 
            powercode.powercode_api_key ?? "";
        document.getElementById("powercodeApiUrl").value = 
            powercode.powercode_api_url ?? "";
    } else {
        console.error("Powercode integration configuration not found in window.config");
    }
});
