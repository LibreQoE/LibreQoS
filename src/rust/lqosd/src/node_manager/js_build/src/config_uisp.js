import {saveConfig, loadConfig, renderConfigMenu} from "./config/config_helper";

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableUisp").checked) {
        const token = document.getElementById("uispToken").value.trim();
        if (!token) {
            alert("API Token is required when UISP integration is enabled");
            return false;
        }

        const url = document.getElementById("uispUrl").value.trim();
        if (!url) {
            alert("UISP URL is required when UISP integration is enabled");
            return false;
        }
        try {
            new URL(url);
        } catch {
            alert("UISP URL must be a valid URL");
            return false;
        }

        const site = document.getElementById("uispSite").value.trim();
        if (!site) {
            alert("UISP Site is required when UISP integration is enabled");
            return false;
        }

        const strategy = document.getElementById("uispStrategy").value.trim();
        if (!strategy) {
            alert("Strategy is required when UISP integration is enabled");
            return false;
        }

        const suspendedStrategy = document.getElementById("uispSuspendedStrategy").value.trim();
        if (!suspendedStrategy) {
            alert("Suspended Strategy is required when UISP integration is enabled");
            return false;
        }

        // Validate numeric fields
        const airmaxCapacity = parseFloat(document.getElementById("uispAirmaxCapacity").value);
        if (isNaN(airmaxCapacity) || airmaxCapacity < 0) {
            alert("Airmax Capacity must be a number greater than or equal to 0");
            return false;
        }

        const ltuCapacity = parseFloat(document.getElementById("uispLtuCapacity").value);
        if (isNaN(ltuCapacity) || ltuCapacity < 0) {
            alert("LTU Capacity must be a number greater than or equal to 0");
            return false;
        }

        const bandwidthOverhead = parseFloat(document.getElementById("uispBandwidthOverhead").value);
        if (isNaN(bandwidthOverhead) || bandwidthOverhead <= 0) {
            alert("Bandwidth Overhead Factor must be a number greater than 0");
            return false;
        }

        const commitMultiplier = parseFloat(document.getElementById("uispCommitMultiplier").value);
        if (isNaN(commitMultiplier) || commitMultiplier <= 0) {
            alert("Commit Bandwidth Multiplier must be a number greater than 0");
            return false;
        }

        const exceptionCpes = document.getElementById("uispExceptionCpes").value.trim();
        if (exceptionCpes) {
            const entries = exceptionCpes.split(',');
            for (const entry of entries) {
                if (!entry.includes(':')) {
                    alert(`Exception CPE entry "${entry}" must be in "cpe:parent" format`);
                    return false;
                }
            }
        }
    }
    return true;
}

function updateConfig() {
    // Update only the uisp_integration section
    // Parse comma-separated strings into arrays
    const excludeSites = document.getElementById("uispExcludeSites").value.trim();
    const excludeSitesArray = excludeSites ? excludeSites.split(',').map(s => s.trim()) : [];

    const squashSites = document.getElementById("uispSquashSites").value.trim();
    const squashSitesArray = squashSites ? squashSites.split(',').map(s => s.trim()) : null;

    const exceptionCpes = document.getElementById("uispExceptionCpes").value.trim();
    const exceptionCpesArray = exceptionCpes ? exceptionCpes.split(',').map(s => {
        const [cpe, parent] = s.split(':').map(part => part.trim());
        return { cpe, parent };
    }) : [];

    const doNotSquashSites = document.getElementById("uispDoNotSquashSites").value.trim();
    const doNotSquashSitesArray = doNotSquashSites ? doNotSquashSites.split(',').map(s => s.trim()) : null;

    // Update the config object
    window.config.uisp_integration = {
        ...(window.config.uisp_integration || {}),  // Preserve existing values
        enable_uisp: document.getElementById("enableUisp").checked,
        token: document.getElementById("uispToken").value.trim(),
        url: document.getElementById("uispUrl").value.trim(),
        site: document.getElementById("uispSite").value.trim(),
        strategy: document.getElementById("uispStrategy").value.trim(),
        suspended_strategy: document.getElementById("uispSuspendedStrategy").value.trim(),
        airmax_capacity: parseFloat(document.getElementById("uispAirmaxCapacity").value),
        ltu_capacity: parseFloat(document.getElementById("uispLtuCapacity").value),
        ipv6_with_mikrotik: document.getElementById("uispIpv6WithMikrotik").checked,
        bandwidth_overhead_factor: parseFloat(document.getElementById("uispBandwidthOverhead").value),
        commit_bandwidth_multiplier: parseFloat(document.getElementById("uispCommitMultiplier").value),
        use_ptmp_as_parent: document.getElementById("uispUsePtmpAsParent").checked,
        ignore_calculated_capacity: document.getElementById("uispIgnoreCalculatedCapacity").checked,
        insecure_ssl: document.getElementById("uispInsecureSsl").checked,
        exclude_sites: excludeSitesArray,
        squash_sites: squashSitesArray && squashSitesArray.length > 0 ? squashSitesArray : null,
        exception_cpes: exceptionCpesArray,
        enable_squashing: document.getElementById("uispEnableSquashing").checked,
        do_not_squash_sites: doNotSquashSitesArray && doNotSquashSitesArray.length > 0 ? doNotSquashSitesArray : null,
    };
}

// Render the configuration menu
renderConfigMenu('uisp');

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.uisp_integration) {
        const uisp = window.config.uisp_integration;
        
        // Boolean fields
        document.getElementById("enableUisp").checked = uisp.enable_uisp ?? false;
        document.getElementById("uispIpv6WithMikrotik").checked = uisp.ipv6_with_mikrotik ?? false;
        document.getElementById("uispUsePtmpAsParent").checked = uisp.use_ptmp_as_parent ?? false;
        document.getElementById("uispIgnoreCalculatedCapacity").checked = uisp.ignore_calculated_capacity ?? false;
        document.getElementById("uispInsecureSsl").checked = uisp.insecure_ssl ?? false;

        // String fields
        document.getElementById("uispToken").value = uisp.token ?? "";
        document.getElementById("uispUrl").value = uisp.url ?? "";
        document.getElementById("uispSite").value = uisp.site ?? "";
        document.getElementById("uispStrategy").value = uisp.strategy ?? "full";
        document.getElementById("uispSuspendedStrategy").value = uisp.suspended_strategy ?? "none";

        // Numeric fields
        document.getElementById("uispAirmaxCapacity").value = uisp.airmax_capacity ?? 0.0;
        document.getElementById("uispLtuCapacity").value = uisp.ltu_capacity ?? 0.0;
        document.getElementById("uispBandwidthOverhead").value = uisp.bandwidth_overhead_factor ?? 1.0;
        document.getElementById("uispCommitMultiplier").value = uisp.commit_bandwidth_multiplier ?? 1.0;

        // New fields
        document.getElementById("uispExcludeSites").value = uisp.exclude_sites?.join(", ") || "";
        document.getElementById("uispSquashSites").value = uisp.squash_sites?.join(", ") || "";
        document.getElementById("uispExceptionCpes").value = uisp.exception_cpes?.map(e => `${e.cpe}:${e.parent}`).join(", ") || "";
        document.getElementById("uispEnableSquashing").checked = uisp.enable_squashing ?? false;
        document.getElementById("uispDoNotSquashSites").value = uisp.do_not_squash_sites?.join(", ") || "";

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
        console.error("UISP integration configuration not found in window.config");
    }
});
