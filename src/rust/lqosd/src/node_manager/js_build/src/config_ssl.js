import {loadConfig, renderConfigMenu} from "./config/config_helper";
import {
    SSL_DISABLE_REDIRECT_DELAY_MS,
    directHttpUrlFromLocation,
    sslDisableRedirectTarget,
} from "./config/ssl_redirect.mjs";
import {renderMarkupText} from "./config/markup_text.mjs";
import {t, tHtml} from "./i18n/index.js";

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
        throw new Error(detail || t("config.ssl.request_failed", {status: response.status}));
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
    document.getElementById("sslCaddyInstalled").textContent = status?.caddy_installed
        ? t("config.ssl.value.yes")
        : t("config.ssl.value.no");
    document.getElementById("sslCaddyfilePresent").textContent = status?.caddyfile_present
        ? t("config.ssl.value.present")
        : t("config.ssl.value.not_present");
    document.getElementById("sslModeLabel").textContent = status?.using_internal_ca
        ? t("config.ssl.mode.local_ca")
        : t("config.ssl.mode.public");
    document.getElementById("sslCaPath").innerHTML = status?.internal_ca_root_certificate
        ? `<code>${escapeHtml(status.internal_ca_root_certificate)}</code>`
        : t("config.ssl.value.not_needed");

    if (status?.enabled) {
        const method = status.using_internal_ca
            ? t("config.ssl.using.local_ca")
            : t("config.ssl.using.public");
        setStatus("success", tHtml("config.ssl.status.enabled", {method}));
    } else if (status?.managed_by_libreqos) {
        setStatus(
            "secondary",
            t("config.ssl.status.disabled_managed"),
        );
    } else {
        setStatus(
            "secondary",
            t("config.ssl.status.disabled_unmanaged"),
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
        html += ` <br><span class="small">${tHtml("config.ssl.action.trust", {path: outcome.internal_ca_root_certificate})}</span>`;
    }
    html += ` <br><span class="small">${tHtml("config.ssl.action.open", {url: outcome.target_url})}</span>`;
    return html;
}

function redirectNotice(targetUrl) {
    return tHtml("config.ssl.redirect_notice", {url: targetUrl});
}

function scheduleRedirect(targetUrl) {
    window.setTimeout(() => {
        window.location.assign(targetUrl);
    }, SSL_DISABLE_REDIRECT_DELAY_MS);
}

async function setupSsl() {
    setResult("info", t("config.ssl.preparing_setup"));
    const outcome = await fetchJson("/local-api/ssl/setup", {
        method: "POST",
        body: JSON.stringify({
            external_hostname: hostnameValue() || null,
        }),
    });
    setResult("success", actionMessage(outcome));
}

async function disableSsl() {
    setResult("info", t("config.ssl.preparing_shutdown"));
    const fallbackTarget = directHttpUrlFromLocation(window.location);
    try {
        const outcome = await fetchJson("/local-api/ssl/disable", {
            method: "POST",
            body: JSON.stringify({}),
        });
        const targetUrl = sslDisableRedirectTarget(outcome, window.location);
        setResult("success", `${actionMessage(outcome)} <br><span class="small">${redirectNotice(targetUrl)}</span>`);
        scheduleRedirect(targetUrl);
    } catch (error) {
        // translatable: false — "https:" is the protocol token compared here, never rendered
        if (window.location.protocol === "https:") {
            setResult(
                "warning",
                `${escapeHtml(error.message)} <br><span class="small">${redirectNotice(fallbackTarget)}</span>`,
            );
            scheduleRedirect(fallbackTarget);
            return;
        }
        throw error;
    }
}

renderConfigMenu("ssl");
// These paragraphs carry a literal example or scheme inside `<code>`, which the
// DOM applier cannot keep: they are empty in the page and rendered here.
renderMarkupText({
    sslHowHostname: tHtml("config.ssl.how.hostname"),
    sslHowIp: tHtml("config.ssl.how.ip"),
    sslHostnameRule: tHtml("config.ssl.hostname.rule"),
});

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
