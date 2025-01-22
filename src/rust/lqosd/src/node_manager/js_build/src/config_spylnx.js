import {loadConfig} from "./config/config_helper";

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.spylnx_integration) {
        const spylnx = window.config.spylnx_integration;
        
        // Boolean field
        document.getElementById("enableSplynx").checked = 
            spylnx.enable_spylnx ?? false;

        // String fields
        document.getElementById("spylnxApiKey").value = 
            spylnx.api_key ?? "";
        document.getElementById("spylnxApiSecret").value = 
            spylnx.api_secret ?? "";
        document.getElementById("spylnxUrl").value = 
            spylnx.url ?? "";
    } else {
        console.error("Splynx integration configuration not found in window.config");
    }
});
