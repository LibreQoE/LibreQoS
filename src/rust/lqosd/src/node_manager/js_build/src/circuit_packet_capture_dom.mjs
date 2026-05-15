import {appendIconText} from "./helpers/safe_dom.mjs";

export function setIconText(target, iconClasses, text) {
    while (target.firstChild) {
        target.removeChild(target.firstChild);
    }
    appendIconText(target, iconClasses, text);
}

export function appendRedactableText(target, text) {
    const span = document.createElement("span");
    span.classList.add("redactable");
    span.textContent = text ?? "";
    target.appendChild(span);
}

export function setPacketCaptureDownloadButton(button, address) {
    setIconText(button, ["fa", "fa-download"], "Download Packet Capture for ");
    appendRedactableText(button, address);
}
