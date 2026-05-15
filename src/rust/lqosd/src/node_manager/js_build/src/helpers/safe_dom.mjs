export function safeRelativeHref(href) {
    const value = String(href ?? "");
    const trimmed = value.trimStart();
    if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed) || trimmed.startsWith("//")) {
        return "#";
    }
    return value;
}

export function simpleLinkRow(href, text, redact=false) {
    let td = document.createElement("td");
    let link = document.createElement("a");
    link.href = safeRelativeHref(href);
    link.textContent = text ?? "";
    if (redact) {
        link.classList.add("redactable");
    }
    td.appendChild(link);
    return td;
}
