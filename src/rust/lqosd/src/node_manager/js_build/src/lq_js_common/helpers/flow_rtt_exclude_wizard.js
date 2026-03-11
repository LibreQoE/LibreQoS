function isProbablyIpv6(ip) {
    return String(ip || "").includes(":");
}

async function copyTextToClipboard(text) {
    const value = String(text ?? "");
    if (!value) return false;

    // Modern API
    try {
        if (navigator?.clipboard?.writeText) {
            await navigator.clipboard.writeText(value);
            return true;
        }
    } catch (e) {
        // Fall back
    }

    // Legacy fallback
    try {
        const textarea = document.createElement("textarea");
        textarea.value = value;
        textarea.setAttribute("readonly", "");
        textarea.style.position = "fixed";
        textarea.style.left = "-1000px";
        textarea.style.top = "-1000px";
        document.body.appendChild(textarea);
        textarea.focus();
        textarea.select();
        const ok = document.execCommand("copy");
        textarea.remove();
        return ok;
    } catch (e) {
        return false;
    }
}

function openConfigFlowsPrefilled(cidr) {
    const value = String(cidr ?? "").trim();
    if (!value) return;
    const url = `/config_flows.html?prefillDoNotTrack=${encodeURIComponent(value)}`;
    const w = window.open(url, "_blank", "noopener,noreferrer");
    if (w === null) {
        window.location.href = url;
    }
}

export function openFlowRttExcludeWizard({ remoteIp, sourceLabel } = {}) {
    const ip = String(remoteIp ?? "").trim();
    if (!ip) {
        alert("No remote IP available for this flow.");
        return;
    }

    const isV6 = isProbablyIpv6(ip);
    const hostCidr = `${ip}/${isV6 ? 128 : 32}`;
    const wideCidr = `${ip}/${isV6 ? 64 : 24}`;

    const modalId = `flowRttExcludeModal_${Date.now()}_${Math.floor(Math.random() * 1_000_000)}`;
    const labelId = `${modalId}_label`;
    const radioName = `${modalId}_scope`;
    let selectedCidr = hostCidr;

    // Modal wrapper
    const modal = document.createElement("div");
    modal.className = "modal fade";
    modal.id = modalId;
    modal.tabIndex = -1;
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-labelledby", labelId);
    modal.setAttribute("aria-hidden", "true");

    // Dialog
    const dialog = document.createElement("div");
    dialog.className = "modal-dialog modal-dialog-centered";
    dialog.setAttribute("role", "document");

    // Content
    const content = document.createElement("div");
    content.className = "modal-content";

    // Header
    const header = document.createElement("div");
    header.className = "modal-header";
    const title = document.createElement("h5");
    title.className = "modal-title";
    title.id = labelId;
    title.textContent = "Exclude RTT for Remote Endpoint";
    const closeX = document.createElement("button");
    closeX.type = "button";
    closeX.className = "btn-close";
    closeX.setAttribute("data-bs-dismiss", "modal");
    closeX.setAttribute("aria-label", "Close");
    header.appendChild(title);
    header.appendChild(closeX);

    // Body
    const body = document.createElement("div");
    body.className = "modal-body";

    const meta = document.createElement("div");
    meta.className = "small";
    const sourceLine = document.createElement("div");
    sourceLine.className = "text-muted";
    sourceLine.textContent = sourceLabel ? `Source: ${sourceLabel}` : "Source: Flow view";
    const remoteLine = document.createElement("div");
    remoteLine.className = "mt-1";
    const remoteLabel = document.createElement("span");
    remoteLabel.className = "text-muted";
    remoteLabel.textContent = "Remote IP: ";
    const remoteCode = document.createElement("code");
    remoteCode.textContent = ip;
    remoteLine.appendChild(remoteLabel);
    remoteLine.appendChild(remoteCode);
    meta.appendChild(sourceLine);
    meta.appendChild(remoteLine);

    const explain = document.createElement("div");
    explain.className = "small mt-3";
    explain.textContent =
        "This helps you add a Flow Tracking → Do Not Track Subnets entry. " +
        "RTT samples where the remote IP matches will be ignored for scoring/heatmaps. " +
        "This is global (affects all circuits). Nothing is saved automatically.";

    const form = document.createElement("div");
    form.className = "mt-3";

    const scopeLabel = document.createElement("div");
    scopeLabel.className = "small fw-semibold mb-2";
    scopeLabel.textContent = "Choose exclusion scope:";

    const hostId = `${modalId}_host`;
    const wideId = `${modalId}_wide`;

    const hostWrap = document.createElement("div");
    hostWrap.className = "form-check";
    const hostRadio = document.createElement("input");
    hostRadio.className = "form-check-input";
    hostRadio.type = "radio";
    hostRadio.name = radioName;
    hostRadio.id = hostId;
    hostRadio.value = hostCidr;
    hostRadio.checked = true;
    hostRadio.addEventListener("change", () => {
        selectedCidr = hostCidr;
        selectedCode.textContent = selectedCidr;
    });
    const hostText = document.createElement("label");
    hostText.className = "form-check-label";
    hostText.htmlFor = hostId;
    hostText.appendChild(document.createTextNode("Only this host (recommended): "));
    const hostCode = document.createElement("code");
    hostCode.textContent = hostCidr;
    hostText.appendChild(hostCode);
    hostWrap.appendChild(hostRadio);
    hostWrap.appendChild(hostText);

    const wideWrap = document.createElement("div");
    wideWrap.className = "form-check mt-2";
    const wideRadio = document.createElement("input");
    wideRadio.className = "form-check-input";
    wideRadio.type = "radio";
    wideRadio.name = radioName;
    wideRadio.id = wideId;
    wideRadio.value = wideCidr;
    wideRadio.checked = false;
    wideRadio.addEventListener("change", () => {
        selectedCidr = wideCidr;
        selectedCode.textContent = selectedCidr;
    });
    const wideText = document.createElement("label");
    wideText.className = "form-check-label";
    wideText.htmlFor = wideId;
    wideText.appendChild(document.createTextNode(`Common wider scope (use cautiously): `));
    const wideCode = document.createElement("code");
    wideCode.textContent = wideCidr;
    wideText.appendChild(wideCode);
    wideWrap.appendChild(wideRadio);
    wideWrap.appendChild(wideText);

    const selected = document.createElement("div");
    selected.className = "small text-muted mt-3";
    selected.appendChild(document.createTextNode("Selected: "));
    const selectedCode = document.createElement("code");
    selectedCode.textContent = selectedCidr;
    selected.appendChild(selectedCode);

    form.appendChild(scopeLabel);
    form.appendChild(hostWrap);
    form.appendChild(wideWrap);
    form.appendChild(selected);

    body.appendChild(meta);
    body.appendChild(explain);
    body.appendChild(form);

    // Footer
    const footer = document.createElement("div");
    footer.className = "modal-footer";

    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.className = "btn btn-outline-secondary";
    copyBtn.textContent = "Copy CIDR";

    const openBtn = document.createElement("button");
    openBtn.type = "button";
    openBtn.className = "btn btn-outline-primary";
    openBtn.textContent = "Open Flow Tracking Config";

    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "btn btn-secondary";
    closeBtn.setAttribute("data-bs-dismiss", "modal");
    closeBtn.textContent = "Close";

    footer.appendChild(copyBtn);
    footer.appendChild(openBtn);
    footer.appendChild(closeBtn);

    content.appendChild(header);
    content.appendChild(body);
    content.appendChild(footer);
    dialog.appendChild(content);
    modal.appendChild(dialog);
    document.body.appendChild(modal);

    const bsModal = new bootstrap.Modal(modal, { focus: true });
    bsModal.show();

    let copyBusy = false;
    copyBtn.addEventListener("click", async () => {
        if (copyBusy) return;
        copyBusy = true;
        const original = copyBtn.textContent;
        const ok = await copyTextToClipboard(selectedCidr);
        copyBtn.textContent = ok ? "Copied!" : "Copy failed";
        setTimeout(() => {
            copyBtn.textContent = original;
            copyBusy = false;
        }, 1500);
    });

    openBtn.addEventListener("click", () => {
        openConfigFlowsPrefilled(selectedCidr);
        bsModal.hide();
    });

    modal.addEventListener("hidden.bs.modal", () => {
        modal.remove();
    });
}

