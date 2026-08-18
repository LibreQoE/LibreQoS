export const DRAFT_KEY = "lqos-network-mode-draft";
export const PENDING_OPERATION_KEY = "lqos-network-mode-pending";

const OPERATIONAL_NETWORK_MODE_KEYS = [
    DRAFT_KEY,
    PENDING_OPERATION_KEY,
];

function storageOf(kind) {
    try {
        return globalThis.window?.[kind] ?? globalThis[kind] ?? null;
    } catch (_) {
        return null;
    }
}

function readJson(storage, key) {
    if (!storage) return null;
    try {
        const raw = storage.getItem(key);
        if (!raw) return null;
        return JSON.parse(raw);
    } catch (_) {
        return null;
    }
}

export function loadNetworkModeState(key) {
    const session = storageOf("sessionStorage");
    const value = readJson(session, key);
    if (value !== null) return value;

    const local = storageOf("localStorage");
    const legacyValue = readJson(local, key);
    if (legacyValue !== null && session) {
        try {
            session.setItem(key, JSON.stringify(legacyValue));
        } catch (_) {}
    }
    if (local) {
        try {
            local.removeItem(key);
        } catch (_) {}
    }
    return legacyValue;
}

export function saveNetworkModeState(key, value) {
    const session = storageOf("sessionStorage");
    if (session) {
        session.setItem(key, JSON.stringify(value));
    }
    removeLegacyNetworkModeState(key);
}

export function clearNetworkModeState(key) {
    const session = storageOf("sessionStorage");
    if (session) {
        session.removeItem(key);
    }
    removeLegacyNetworkModeState(key);
}

export function clearOperationalNetworkModeStorage() {
    OPERATIONAL_NETWORK_MODE_KEYS.forEach(clearNetworkModeState);
}

function removeLegacyNetworkModeState(key) {
    const local = storageOf("localStorage");
    if (local) {
        local.removeItem(key);
    }
}
