import {t} from "../i18n/index.js";

// One entry per topology source. Product names stay literal in both locales;
// the labels and descriptions operators read are catalog text.
export const TOPOLOGY_SOURCES = [
    {
        value: "uisp",
        label: "UISP",
        integration: true,
        domId: "enableUisp",
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: t("config.common.topology_source.producer", {integration: "UISP"}),
    },
    {
        value: "splynx",
        label: "Splynx",
        integration: true,
        domId: "enableSplynx",
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: t("config.common.topology_source.producer", {integration: "Splynx"}),
    },
    {
        value: "netzur",
        label: "Netzur",
        integration: true,
        domId: "enableNetzur",
        modes: ["full"],
        description: t("config.common.topology_source.producer", {integration: "Netzur"}),
    },
    {
        value: "visp",
        label: "VISP",
        integration: true,
        domId: "enableVisp",
        modes: ["full"],
        description: t("config.common.topology_source.producer", {integration: "VISP"}),
    },
    {
        value: "sonar",
        label: "Sonar",
        integration: true,
        domId: "enableSonar",
        modes: ["flat", "full"],
        description: t("config.common.topology_source.producer", {integration: "Sonar"}),
    },
    {
        value: "powercode",
        label: "Powercode",
        integration: true,
        domId: "enablePowercode",
        modes: ["full"],
        description: t("config.common.topology_source.producer", {integration: "Powercode"}),
    },
    {
        value: "wispgate",
        label: "WispGate",
        integration: true,
        domId: "enableWispgate",
        modes: ["full"],
        description: t("config.common.topology_source.producer", {integration: "WispGate"}),
    },
    {
        value: "shaped_devices",
        label: t("config.common.topology_source.shaped_devices"),
        integration: false,
        domId: null,
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: t("config.common.topology_source.shaped_devices_help"),
    },
    {
        value: "libreqos_network",
        label: t("config.common.topology_source.libreqos_network"),
        integration: false,
        domId: null,
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: t("config.common.topology_source.libreqos_network_help"),
    },
];

const TOPOLOGY_SOURCE_BY_VALUE = new Map(TOPOLOGY_SOURCES.map((entry) => [entry.value, entry]));

export function topologySource(config) {
    const value = config?.topology?.source;
    return typeof value === "string" && value.trim().length > 0 ? value : null;
}

export function topologySourceDefinition(source) {
    return TOPOLOGY_SOURCE_BY_VALUE.get(source) || null;
}

export function topologySourceLabel(source) {
    return TOPOLOGY_SOURCE_BY_VALUE.get(source)?.label || null;
}

export function topologyConflictMessage() {
    if (typeof window === "undefined") {
        return "";
    }
    return window.configView?.topology_source_conflict?.message || "";
}

export function activeTopologySourceIntegrations(config) {
    const definition = topologySourceDefinition(topologySource(config));
    return definition?.integration ? [definition.label] : [];
}

export function topologyEditorsLocked(config) {
    const source = topologySource(config);
    if (source === "shaped_devices") {
        return Boolean(topologyConflictMessage());
    }
    return true;
}

export function topologyEditorsLockMessage(config) {
    const source = topologySource(config);
    const conflict = topologyConflictMessage();
    if (source === "shaped_devices") {
        if (!conflict) return "";
        // The server's conflict text is English and already self-describing;
        // it is only re-prefixed when the payload arrives without one.
        // translatable: false — "topology source conflict" is the server message prefix this check matches verbatim
        return conflict.toLowerCase().startsWith("topology source conflict")
            ? conflict
            : t("config.common.topology_source.conflict", {detail: conflict});
    }
    if (!source) {
        return conflict
            ? t("config.common.topology_source.none_selected_detail", {detail: conflict})
            : t("config.common.topology_source.none_selected");
    }
    const base = source === "libreqos_network"
        ? t("config.common.topology_source.from_drop")
        : t("config.common.topology_source.from_integration", {
            integration: topologySourceLabel(source) || source,
        });
    return conflict ? `${base} ${conflict}` : base;
}

export function buildTopologyConfigUpdate(config, source, compileMode) {
    return {
        ...(config?.topology || {}),
        source,
        compile_mode: compileMode,
    };
}
