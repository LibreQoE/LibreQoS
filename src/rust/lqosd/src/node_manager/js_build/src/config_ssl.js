import {loadConfig, renderConfigMenu} from "./config/config_helper";

function escapeHtml(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function setResult(kind, message) {
    const holder = document.getElementById("sslResult");
    if (!holder) return;
    holder.className = `alert alert-${kind} mt-3`;
    holder.classList.remove("d-none");
    holder.innerHTML = message;
    holder.setAttribute("tabindex", "-1");
    holder.focus();
}

function setStatus(kind, message) {
    const holder = document.getElementById("sslStatusAlert");
    if (!holder) return;
    holder.className = `alert alert-${kind} mb-3`;
    holder.innerHTML = message;
}

function hostnameValue() {
    return String(document.getElementById("externalHostname")?.value || "").trim();
}

async function fetchJson(url, options = {}) {
    const response = await fetch(url, {
        credentials: "same-origin",
        headers: {
            "Content-Type": "application/json",
            ...(options.headers || {}),
        },
        ...options,
    });

    if (!response.ok) {
        const detail = (await response.text().catch(() => "")).trim();
        throw new Error(detail || `Request failed with HTTP ${response.status}.`);
    }

    return response.json();
}

function renderStatus(status) {
    const ssl = window.config?.ssl || {};
    const hostnameInput = document.getElementById("externalHostname");
    if (hostnameInput && !hostnameInput.value.trim()) {
        hostnameInput.value = ssl.external_hostname || "";
    }

    const webserverBlocked = !!window.config?.disable_webserver;
    document.getElementById("sslWebserverBlocked")?.classList.toggle("d-none", !webserverBlocked);
    document.getElementById("setupSslButton").disabled = webserverBlocked;

    document.getElementById("sslTargetUrl").innerHTML = status?.target_url
        ? `<a href="${escapeHtml(status.target_url)}">${escapeHtml(status.target_url)}</a>`
        : "-";
    document.getElementById("sslCaddyInstalled").textContent = status?.caddy_installed ? "Yes" : "No";
    document.getElementById("sslCaddyfilePresent").textContent = status?.caddyfile_present ? "Present" : "Not present";
    document.getElementById("sslModeLabel").textContent = status?.using_internal_ca
        ? "Local certificate authority"
        : "Let's Encrypt / public certificate";
    document.getElementById("sslCaPath").innerHTML = status?.internal_ca_root_certificate
        ? `<code>${escapeHtml(status.internal_ca_root_certificate)}</code>`
        : "Not needed";

    if (status?.enabled) {
        setStatus(
            "success",
            `HTTPS is enabled. LibreQoS is using <strong>${status.using_internal_ca ? "a local certificate authority" : "a public certificate flow"}</strong>.`,
        );
    } else if (status?.managed_by_libreqos) {
        setStatus(
            "secondary",
            "HTTPS is currently disabled. LibreQoS still owns the optional Caddy setup and can turn it back on from this page.",
        );
    } else {
        setStatus(
            "secondary",
            "HTTPS is currently disabled. Use Setup SSL to install Caddy and move the WebUI behind HTTPS.",
        );
    }
    document.getElementById("disableSslButton").disabled = !status?.managed_by_libreqos;
}

async function refreshStatus() {
    const status = await fetchJson("/local-api/ssl/status", { method: "GET", headers: {} });
    renderStatus(status);
    return status;
}

function actionMessage(outcome) {
    let html = escapeHtml(outcome.message);
    if (outcome.internal_ca_root_certificate) {
        html += ` <br><span class="small">Trust <code>${escapeHtml(outcome.internal_ca_root_certificate)}</code> on operator workstations if the browser warns.</span>`;
    }
    html += ` <br><span class="small">Open <a href="${escapeHtml(outcome.target_url)}">${escapeHtml(outcome.target_url)}</a> after the service switch.</span>`;
    return html;
}

async function setupSsl() {
    setResult("info", "Preparing HTTPS setup...");
    const outcome = await fetchJson("/local-api/ssl/setup", {
        method: "POST",
        body: JSON.stringify({
            external_hostname: hostnameValue() || null,
        }),
    });
    setResult("success", actionMessage(outcome));
}

async function disableSsl() {
    setResult("info", "Preparing HTTPS shutdown...");
    const outcome = await fetchJson("/local-api/ssl/disable", {
        method: "POST",
        body: JSON.stringify({}),
    });
    setResult("success", actionMessage(outcome));
}

renderConfigMenu("ssl");

loadConfig(async () => {
    const hostnameInput = document.getElementById("externalHostname");
    if (hostnameInput) {
        hostnameInput.value = window.config?.ssl?.external_hostname || "";
    }

    document.getElementById("refreshSslStatusButton")?.addEventListener("click", () => {
        refreshStatus().catch((error) => setResult("danger", escapeHtml(error.message)));
    });
    document.getElementById("setupSslButton")?.addEventListener("click", () => {
        setupSsl().catch((error) => setResult("danger", escapeHtml(error.message)));
    });
    document.getElementById("disableSslButton")?.addEventListener("click", () => {
        disableSsl().catch((error) => setResult("danger", escapeHtml(error.message)));
    });

    try {
        await refreshStatus();
    } catch (error) {
        setStatus("danger", escapeHtml(error.message));
    }
});
