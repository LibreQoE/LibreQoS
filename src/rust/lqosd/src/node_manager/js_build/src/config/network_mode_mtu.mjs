import {t} from "../i18n/index.js";

export const MTU_MIN = 576;
export const MTU_MAX = 9216;

// The bounds reach the message as strings: `t()` formats number parameters with
// Intl, which would render 9216 as "9.216" in Spanish, and an MTU is a literal.
export function parseOptionalMtu(rawValue) {
    const trimmed = String(rawValue ?? "").trim();
    if (!trimmed) {
        return { ok: true, value: null, error: "" };
    }
    if (!/^\d+$/.test(trimmed)) {
        return {
            ok: false,
            value: null,
            error: t("config.interface.error.mtu_whole", {min: String(MTU_MIN), max: String(MTU_MAX)}),
        };
    }
    const value = Number.parseInt(trimmed, 10);
    if (value < MTU_MIN || value > MTU_MAX) {
        return {
            ok: false,
            value: null,
            error: t("config.interface.error.mtu_range", {min: String(MTU_MIN), max: String(MTU_MAX)}),
        };
    }
    return { ok: true, value, error: "" };
}

export function hydrateDraftMtu(draft, liveConfig) {
    if (!draft) return null;
    const hydrated = JSON.parse(JSON.stringify(draft));
    if (hydrated.bridge && hydrated.bridge.mtu === undefined) {
        hydrated.bridge.mtu = liveConfig?.bridge?.mtu ?? null;
    }
    if (hydrated.single_interface && hydrated.single_interface.mtu === undefined) {
        hydrated.single_interface.mtu = liveConfig?.single_interface?.mtu ?? null;
    }
    return hydrated;
}
