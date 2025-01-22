import {loadConfig} from "./config/config_helper";

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

        // String fields
        document.getElementById("uispToken").value = uisp.token ?? "";
        document.getElementById("uispUrl").value = uisp.url ?? "";
        document.getElementById("uispSite").value = uisp.site ?? "";
        document.getElementById("uispStrategy").value = uisp.strategy ?? "";
        document.getElementById("uispSuspendedStrategy").value = uisp.suspended_strategy ?? "";

        // Numeric fields
        document.getElementById("uispAirmaxCapacity").value = uisp.airmax_capacity ?? 0.0;
        document.getElementById("uispLtuCapacity").value = uisp.ltu_capacity ?? 0.0;
        document.getElementById("uispBandwidthOverhead").value = uisp.bandwidth_overhead_factor ?? 1.0;
        document.getElementById("uispCommitMultiplier").value = uisp.commit_bandwidth_multiplier ?? 1.0;
    } else {
        console.error("UISP integration configuration not found in window.config");
    }
});
