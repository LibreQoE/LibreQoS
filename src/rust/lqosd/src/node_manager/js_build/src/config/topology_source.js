export const TOPOLOGY_SOURCES = [
    {
        value: "uisp",
        label: "UISP",
        integration: true,
        domId: "enableUisp",
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: "UISP is the topology producer.",
    },
    {
        value: "splynx",
        label: "Splynx",
        integration: true,
        domId: "enableSplynx",
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: "Splynx is the topology producer.",
    },
    {
        value: "netzur",
        label: "Netzur",
        integration: true,
        domId: "enableNetzur",
        modes: ["full"],
        description: "Netzur is the topology producer.",
    },
    {
        value: "visp",
        label: "VISP",
        integration: true,
        domId: "enableVisp",
        modes: ["full"],
        description: "VISP is the topology producer.",
    },
    {
        value: "sonar",
        label: "Sonar",
        integration: true,
        domId: "enableSonar",
        modes: ["flat", "full"],
        description: "Sonar is the topology producer.",
    },
    {
        value: "powercode",
        label: "Powercode",
        integration: true,
        domId: "enablePowercode",
        modes: ["full"],
        description: "Powercode is the topology producer.",
    },
    {
        value: "wispgate",
        label: "WispGate",
        integration: true,
        domId: "enableWispgate",
        modes: ["full"],
        description: "WispGate is the topology producer.",
    },
    {
        value: "shaped_devices",
        label: "Shaped devices",
        integration: false,
        domId: null,
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: "Use the manual ShapedDevices.csv + network.json pair.",
    },
    {
        value: "libreqos_network",
        label: "LibreQoS network",
        integration: false,
        domId: null,
        modes: ["flat", "ap_only", "ap_site", "full"],
        description: "Use the libreqos_network.json watched drop.",
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
        return conflict.toLowerCase().startsWith("topology source conflict")
            ? conflict
            : `Topology source conflict: ${conflict}`;
    }
    if (!source) {
        return conflict
            ? `No topology source is selected. ${conflict}`
            : "No topology source is selected.";
    }
    const base = source === "libreqos_network"
        ? "Topology is produced by the libreqos_network.json drop."
        : `Topology is produced by ${topologySourceLabel(source) || source}. Change Topology Source to edit manually.`;
    return conflict ? `${base} ${conflict}` : base;
}

export function buildTopologyConfigUpdate(config, source, compileMode) {
    return {
        ...(config?.topology || {}),
        source,
        compile_mode: compileMode,
    };
}
