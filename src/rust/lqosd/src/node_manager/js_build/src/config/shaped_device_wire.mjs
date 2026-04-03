export function parseIpv4Address(text) {
    const parts = text.split(".");
    if (parts.length !== 4) return null;
    const octets = [];
    for (const part of parts) {
        if (!/^\d{1,3}$/.test(part)) return null;
        const octet = parseInt(part, 10);
        if (octet < 0 || octet > 255) return null;
        octets.push(octet);
    }
    return octets;
}

function expandIpv4Tail(text) {
    if (!text.includes(".")) return text;
    const idx = text.lastIndexOf(":");
    if (idx === -1) return null;
    const tail = parseIpv4Address(text.slice(idx + 1));
    if (!tail) return null;
    const high = ((tail[0] << 8) | tail[1]).toString(16);
    const low = ((tail[2] << 8) | tail[3]).toString(16);
    return text.slice(0, idx) + ":" + high + ":" + low;
}

export function parseIpv6Address(text) {
    let input = (text || "").trim().toLowerCase();
    if (!input || input.includes("%")) return null;
    input = expandIpv4Tail(input);
    if (!input) return null;

    const doubleColon = input.match(/::/g) || [];
    if (doubleColon.length > 1) return null;

    let head = [];
    let tail = [];
    if (input.includes("::")) {
        const parts = input.split("::");
        head = parts[0] ? parts[0].split(":") : [];
        tail = parts[1] ? parts[1].split(":") : [];
    } else {
        head = input.split(":");
    }

    const validGroup = (group) => /^[0-9a-f]{1,4}$/.test(group);
    if (!head.every(validGroup) || !tail.every(validGroup)) return null;

    if (!input.includes("::") && head.length !== 8) return null;
    if (input.includes("::") && head.length + tail.length >= 8) return null;

    const groups = input.includes("::")
        ? [
            ...head,
            ...new Array(8 - head.length - tail.length).fill("0"),
            ...tail,
        ]
        : head;
    if (groups.length !== 8) return null;

    const bytes = [];
    for (const group of groups) {
        const value = parseInt(group, 16);
        if (Number.isNaN(value) || value < 0 || value > 0xffff) return null;
        bytes.push((value >> 8) & 0xff, value & 0xff);
    }
    return bytes;
}

export function parseIpInput(text, family) {
    if (!text) return [];
    const defaultPrefix = family === 6 ? 128 : 32;
    const tokens = text.split(/[\n,]+/);
    const result = [];
    tokens.forEach((token) => {
        const trimmed = token.trim();
        if (!trimmed) return;
        const parts = trimmed.split("/");
        const addr = parts[0].trim();
        if (!addr) return;
        let prefix = defaultPrefix;
        if (parts.length > 1 && parts[1].trim().length > 0) {
            const parsed = parseInt(parts[1].trim(), 10);
            if (!Number.isNaN(parsed)) prefix = parsed;
        }
        const encoded = family === 6 ? parseIpv6Address(addr) : parseIpv4Address(addr);
        result.push([encoded || addr, prefix]);
    });
    return result;
}
