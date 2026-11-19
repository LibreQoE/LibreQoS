import {applyDomTranslations, t} from "./i18n/index.js";
import {initLanguageSwitcher} from "./i18n/switcher.mjs";

// The standalone pages load no shell bundle, so this page builds its own
// language control from the generated locale list and translates its own
// `data-i18n` markup; English stays inline as the fallback.
initLanguageSwitcher({containerId: "languageControl"});
applyDomTranslations();

// `/firstLogin` answers with a machine `reason` plus an English `message`. The
// reason selects the sentence, so the page can answer in the active locale; a
// reason this page does not know falls back to the server's text, then to the
// generic sentence.
function errorText(reason, serverMessage) {
    switch (reason) {
        case "invalid_setup_code":
            return t("first_run.error_invalid_setup_code");
        case "rate_limited":
            return t("first_run.error_rate_limited");
        case "host_not_allowed":
            return t("first_run.error_host_not_allowed");
        case "first_run_unavailable":
            return t("first_run.error_unavailable");
        case "first_run_code":
            return t("first_run.error_code_consume");
        case "auth_corrupt":
            return t("first_run.error_corrupt");
        case "session_error":
            return t("first_run.error_session");
        default:
            return serverMessage || t("first_run.error_generic");
    }
}

$("#btnCreateUser").on('click', () => {
    let username = ($("#username").val() || "").trim();
    let password = $("#password").val();
    let setupCode = ($("#setupCode").val() || "").trim();
    if (username === "") {
        alert(t("common.auth.username_required"));
        return;
    }
    if (password === "") {
        alert(t("common.auth.password_required"));
        return;
    }
    if (setupCode === "") {
        alert(t("first_run.setup_code_required"));
        return;
    }

    let login = {
        username: username,
        password: password,
        setup_code: setupCode
    }
    $.ajax({
        type: "POST",
        url: "/firstLogin",
        data: JSON.stringify(login),
        contentType: 'application/json',
        beforeSend: () => {
            $("#firstRunError").removeClass("show").addClass("d-none");
        },
        success: () => {
            window.location.href = "/index.html";
        },
        error: (xhr) => {
            const response = xhr && xhr.responseJSON ? xhr.responseJSON : {};
            const reason = response.reason || "";
            if (reason === "already_configured") {
                window.location.href = "/login.html";
                return;
            }

            $("#firstRunErrorText").text(errorText(reason, response.message));
            $("#firstRunError").removeClass("d-none").addClass("show");
        }
    })
});
