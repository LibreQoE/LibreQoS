import {applyDomTranslations, t} from "./i18n/index.js";
import {initLanguageSwitcher} from "./i18n/switcher.mjs";

// The standalone pages load no shell bundle, so this page builds its own
// language control from the generated locale list and translates its own
// `data-i18n` markup; English stays inline as the fallback.
initLanguageSwitcher({containerId: "languageControl"});
applyDomTranslations();

$("#btnLogin").on('click', () => {
    let username = ($("#username").val() || "").trim();
    let password = $("#password").val();
    if (username === "") {
        alert(t("common.auth.username_required"));
        return;
    }
    if (password === "") {
        alert(t("common.auth.password_required"));
        return;
    }

    let login = {
        username: username,
        password: password
    }

    $.ajax({
        type: "POST",
        url: "/doLogin",
        data: JSON.stringify(login),
        contentType: 'application/json',
        beforeSend: () => {
            $("#loginErrorText").text(t("login.error_default"));
            $("#loginError").removeClass("show").addClass("d-none");
        },
        success: () => {
            window.location.href = "/index.html";
        },
        error: (xhr) => {
            const response = xhr && xhr.responseJSON ? xhr.responseJSON : {};
            const reason = response.reason || "";
            if (reason === "first_run_required") {
                window.location.href = "/first-run.html";
                return;
            }

            if (reason === "auth_corrupt") {
                $("#loginErrorText").text(response.message || t("login.error_corrupt"));
            } else if (reason === "invalid_credentials") {
                $("#loginErrorText").text(response.message || t("login.error_invalid_credentials"));
            } else {
                $("#loginErrorText").text(t("login.error_default"));
            }
            $("#loginError").removeClass("d-none").addClass("show");
        }
    })
});

// Add keypress handler for Enter key
// translatable: false — "#username, #password" is the jQuery selector for the two login fields, not a label
$('#username, #password').on('keypress', function(e) {
    if (e.which === 13) {
        e.preventDefault();
        $('#btnLogin').click();
    }
});

// Hide error when typing
$('#username, #password').on('input', function() {
    $("#loginError").fadeOut();
});
