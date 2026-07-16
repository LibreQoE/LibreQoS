export function generateLocalApiToken(cryptoApi = globalThis.crypto) {
    if (!cryptoApi || typeof cryptoApi.getRandomValues !== "function") {
        throw new Error("Secure token generation is unavailable in this browser.");
    }

    const bytes = new Uint8Array(32);
    cryptoApi.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function copyLocalApiToken(
    input,
    clipboard = globalThis.navigator?.clipboard,
    documentApi = globalThis.document,
    secureContext = globalThis.isSecureContext,
) {
    const token = input.value.trim();
    if (!token) {
        throw new Error("Generate or enter a local API bearer token first.");
    }

    if (secureContext && clipboard?.writeText) {
        await clipboard.writeText(token);
        return;
    }

    const originalType = input.type;
    const previousFocus = documentApi?.activeElement;
    input.type = "text";
    let copied = false;
    try {
        input.focus();
        input.select();
        copied = documentApi?.execCommand?.("copy") === true;
    } finally {
        input.type = originalType;
        previousFocus?.focus?.();
    }
    if (!copied) {
        throw new Error("Unable to copy the token. Reveal it and copy it manually.");
    }
}
