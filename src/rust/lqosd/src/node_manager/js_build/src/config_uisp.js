import {
    bindSecretField,
    loadConfig,
    renderConfigMenu,
    saveConfig,
    secretWillExistAfterSave,
} from "./config/config_helper";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

function currentTopologyMode() {
    return (
        window.config?.topology?.compile_mode
        ?? document.getElementById("uispStrategy").value.trim()
    );
}

function rootSiteRequiredForMode(mode) {
    return mode === "full" || mode === "full2";
}

function updateRootSiteRequirement() {
    const required = rootSiteRequiredForMode(currentTopologyMode());
    const siteInput = document.getElementById("uispSite");
    const siteLabel = document.getElementById("uispSiteLabel");
    const siteHelp = document.getElementById("uispSiteHelp");

    siteInput.required = required;
    siteLabel.textContent = required
        ? t("config.uisp.site")
        : t("config.uisp.site_optional");
    siteHelp.textContent = required
        ? t("config.uisp.site_help_required")
        : t("config.uisp.site_help_optional");
}

function validateConfig() {
    // Validate required fields when enabled
    if (document.getElementById("enableUisp").checked) {
        if (!secretWillExistAfterSave("uisp_integration", "token", "uispToken")) {
            alert(t("config.uisp.validation.token_required"));
            return false;
        }

        const url = document.getElementById("uispUrl").value.trim();
        if (!url) {
            alert(t("config.uisp.validation.url_required"));
            return false;
        }
        try {
            new URL(url);
        } catch {
            alert(t("config.uisp.validation.url_invalid"));
            return false;
        }

        const site = document.getElementById("uispSite").value.trim();
        if (rootSiteRequiredForMode(currentTopologyMode()) && !site) {
            alert(t("config.uisp.validation.site_required"));
            return false;
        }

        const suspendedStrategy = document.getElementById("uispSuspendedStrategy").value.trim();
        if (!suspendedStrategy) {
            alert(t("config.uisp.validation.suspended_required"));
            return false;
        }

        // Validate numeric fields
        const airmaxCapacity = parseFloat(document.getElementById("uispAirmaxCapacity").value);
        if (isNaN(airmaxCapacity) || airmaxCapacity < 0) {
            alert(t("config.uisp.validation.airmax_capacity"));
            return false;
        }

        const airmaxFlexibleFrameDownloadRatio = parseFloat(document.getElementById("uispAirmaxFlexibleFrameDownloadRatio").value);
        if (isNaN(airmaxFlexibleFrameDownloadRatio) || airmaxFlexibleFrameDownloadRatio <= 0 || airmaxFlexibleFrameDownloadRatio >= 1) {
            alert(t("config.uisp.validation.flexible_ratio"));
            return false;
        }

        const ltuCapacity = parseFloat(document.getElementById("uispLtuCapacity").value);
        if (isNaN(ltuCapacity) || ltuCapacity < 0) {
            alert(t("config.uisp.validation.ltu_capacity"));
            return false;
        }

        const bandwidthOverhead = parseFloat(document.getElementById("uispBandwidthOverhead").value);
        if (isNaN(bandwidthOverhead) || bandwidthOverhead <= 0) {
            alert(t("config.uisp.validation.overhead_factor"));
            return false;
        }

        const commitMultiplier = parseFloat(document.getElementById("uispCommitMultiplier").value);
        if (isNaN(commitMultiplier) || commitMultiplier <= 0) {
            alert(t("config.uisp.validation.commit_multiplier"));
            return false;
        }

        const exceptionCpes = document.getElementById("uispExceptionCpes").value.trim();
        if (exceptionCpes) {
            const entries = exceptionCpes.split(',');
            for (const entry of entries) {
                if (!entry.includes(':')) {
                    alert(t("config.uisp.validation.exception_cpe", {entry: entry}));
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
    const existingUisp = { ...(window.config.uisp_integration || {}) };

    const excludeSites = document.getElementById("uispExcludeSites").value.trim();
    const excludeSitesArray = excludeSites ? excludeSites.split(',').map(s => s.trim()) : [];

    const exceptionCpes = document.getElementById("uispExceptionCpes").value.trim();
    const exceptionCpesArray = exceptionCpes ? exceptionCpes.split(',').map(s => {
        const [cpe, parent] = s.split(':').map(part => part.trim());
        return { cpe, parent };
    }) : [];

    const doNotSquashSites = document.getElementById("uispDoNotSquashSites").value.trim();
    const doNotSquashSitesArray = doNotSquashSites ? doNotSquashSites.split(',').map(s => s.trim()) : null;

    // Update the config object
    window.config.uisp_integration = {
        ...existingUisp,
        enable_uisp: document.getElementById("enableUisp").checked,
        token: document.getElementById("uispToken").value.trim(),
        url: document.getElementById("uispUrl").value.trim(),
        site: document.getElementById("uispSite").value.trim(),
        strategy: window.config.topology?.compile_mode ?? document.getElementById("uispStrategy").value.trim(),
        suspended_strategy: document.getElementById("uispSuspendedStrategy").value.trim(),
        airmax_capacity: parseFloat(document.getElementById("uispAirmaxCapacity").value),
        airmax_flexible_frame_download_ratio: parseFloat(document.getElementById("uispAirmaxFlexibleFrameDownloadRatio").value),
        ltu_capacity: parseFloat(document.getElementById("uispLtuCapacity").value),
        ipv6_with_mikrotik: document.getElementById("uispIpv6WithMikrotik").checked,
        bandwidth_overhead_factor: parseFloat(document.getElementById("uispBandwidthOverhead").value),
        commit_bandwidth_multiplier: parseFloat(document.getElementById("uispCommitMultiplier").value),
        ignore_calculated_capacity: document.getElementById("uispIgnoreCalculatedCapacity").checked,
        insecure_ssl: document.getElementById("uispInsecureSsl").checked,
        exclude_sites: excludeSitesArray,
        exception_cpes: exceptionCpesArray,
        do_not_squash_sites: doNotSquashSitesArray && doNotSquashSitesArray.length > 0 ? doNotSquashSitesArray : null,
    };
}

// Render the configuration menu
renderConfigMenu('uisp');
// These paragraphs wrap a literal URL or format token in `<code>`, which the DOM
// applier cannot keep: they are empty in the page and rendered here.
renderMarkupText({
    uispUrlHelp: tHtml("config.uisp.url_help"),
    uispCompileModeHelp: tHtml("config.uisp.compile_mode_help"),
    uispFlexibleRatioHelp: tHtml("config.uisp.flexible_ratio_help"),
    uispExceptionCpesHelp: tHtml("config.uisp.exception_cpes_help"),
});

loadConfig(() => {
    // window.config now contains the configuration.
    // Populate form fields with config values
    if (window.config && window.config.uisp_integration) {
        const uisp = window.config.uisp_integration;
        
        // Boolean fields
        document.getElementById("enableUisp").checked = uisp.enable_uisp ?? false;
        document.getElementById("uispIpv6WithMikrotik").checked = uisp.ipv6_with_mikrotik ?? false;
        document.getElementById("uispIgnoreCalculatedCapacity").checked = uisp.ignore_calculated_capacity ?? false;
        document.getElementById("uispInsecureSsl").checked = uisp.insecure_ssl ?? false;

        // String fields
        document.getElementById("uispUrl").value = uisp.url ?? "";
        document.getElementById("uispSite").value = uisp.site ?? "";
        document.getElementById("uispStrategy").value = window.config.topology?.compile_mode ?? uisp.strategy ?? "ap_site";
        document.getElementById("uispSuspendedStrategy").value = uisp.suspended_strategy ?? "none";
        updateRootSiteRequirement();

        // Numeric fields
        document.getElementById("uispAirmaxCapacity").value = uisp.airmax_capacity ?? 1.0;
        document.getElementById("uispAirmaxFlexibleFrameDownloadRatio").value = uisp.airmax_flexible_frame_download_ratio ?? 0.8;
        document.getElementById("uispLtuCapacity").value = uisp.ltu_capacity ?? 1.0;
        document.getElementById("uispBandwidthOverhead").value = uisp.bandwidth_overhead_factor ?? 1.0;
        document.getElementById("uispCommitMultiplier").value = uisp.commit_bandwidth_multiplier ?? 1.0;

        // New fields
        document.getElementById("uispExcludeSites").value = uisp.exclude_sites?.join(", ") || "";
        document.getElementById("uispExceptionCpes").value = uisp.exception_cpes?.map(e => `${e.cpe}:${e.parent}`).join(", ") || "";
        document.getElementById("uispDoNotSquashSites").value = uisp.do_not_squash_sites?.join(", ") || "";

        bindSecretField({
            section: "uisp_integration",
            field: "token",
            inputId: "uispToken",
            statusId: "uispTokenStatus",
            clearButtonId: "clearUispToken",
        });

        document.getElementById("uispStrategy").addEventListener("change", updateRootSiteRequirement);

        // Add save button click handler
        document.getElementById('saveButton').addEventListener('click', () => {
            if (validateConfig()) {
                updateConfig();
                saveConfig(() => {
                    alert(t("config.uisp.saved"));
                });
            }
        });
    } else {
        // translatable: false — "UISP integration configuration not found in window.config" is a console diagnostic, never rendered
        console.error("UISP integration configuration not found in window.config");
    }
});
