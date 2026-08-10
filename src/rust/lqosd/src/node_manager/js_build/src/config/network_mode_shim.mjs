export function bridgeEligibilityField(modeKey, compatibilityShimEnabled) {
    if (modeKey === "single") return "single_interface_eligible";
    return compatibilityShimEnabled ? "compatibility_shim_eligible" : "bridge_eligible";
}

export function normalizedBridgeFlags(useXdpBridge, compatibilityShim) {
    const shimEnabled = Boolean(compatibilityShim);
    return {
        use_xdp_bridge: shimEnabled || Boolean(useXdpBridge),
        compatibility_shim: shimEnabled,
    };
}

export function usesManualXdpWorkflow(config) {
    return Boolean(config?.bridge?.use_xdp_bridge);
}

export async function saveManualXdpConfiguration({
    candidate,
    configState,
    persistConfig,
    clearDraft,
    notifySaved,
    refresh,
}) {
    const previousConfig = configState.config;
    configState.config = candidate;
    try {
        await new Promise((resolve, reject) => persistConfig(resolve, reject));
    } catch (error) {
        configState.config = previousConfig;
        throw error;
    }
    clearDraft();
    notifySaved();
    try {
        await refresh();
        return { refreshError: null };
    } catch (refreshError) {
        return { refreshError };
    }
}
