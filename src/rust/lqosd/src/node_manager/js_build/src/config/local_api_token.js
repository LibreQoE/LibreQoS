export const MAX_LOCAL_API_KEYS = 16;

export function validateLocalApiKeyName(name, existingNames = []) {
    const trimmed = String(name ?? "").trim();
    if (!trimmed) return { ok: false, message: "Enter a name for this API key." };
    if (Array.from(trimmed).length > 64) {
        return { ok: false, message: "API key names cannot exceed 64 characters." };
    }
    const normalized = trimmed.toLocaleLowerCase();
    if (existingNames.some((entry) => String(entry).trim().toLocaleLowerCase() === normalized)) {
        return { ok: false, message: "An API key with that name already exists." };
    }
    return { ok: true, name: trimmed };
}

export function abbreviateLocalApiKeyId(id) {
    const value = String(id ?? "");
    return value.length > 8 ? `${value.slice(0, 8)}…` : value;
}

export function formatLocalApiKeyCreatedAt(createdAtUnix, locale) {
    const seconds = Number(createdAtUnix);
    if (!Number.isFinite(seconds) || seconds <= 0) return "Unknown";
    return new Date(seconds * 1000).toLocaleString(locale);
}

export function clearLocalApiKeyInput(input) {
    input.value = "";
    input.type = "password";
}

export function setLocalApiKeyCreationPending(confirmButton, modalElement, status, pending) {
    confirmButton.disabled = pending;
    if (pending) {
        confirmButton.setAttribute("aria-busy", "true");
    } else {
        confirmButton.removeAttribute("aria-busy");
    }
    for (const button of modalElement.querySelectorAll('[data-bs-dismiss="modal"]')) {
        button.disabled = pending;
    }
    status.className = "small text-secondary mt-3";
    status.textContent = pending ? "Generating the API key…" : "";
}

export async function copyLocalApiToken(
    input,
    clipboard = globalThis.navigator?.clipboard,
    documentApi = globalThis.document,
    secureContext = globalThis.isSecureContext,
) {
    const token = input.value.trim();
    if (!token) throw new Error("No generated API key is available to copy.");
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
    if (!copied) throw new Error("Unable to copy the API key. Reveal it and copy it manually.");
}
