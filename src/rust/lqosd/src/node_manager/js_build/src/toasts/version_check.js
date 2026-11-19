import {createBootstrapToast} from "../lq_js_common/helpers/toasts";
import {t} from "../i18n/index.js";

export function checkForUpgrades() {
    setTimeout(() => {
        if (window.newVersion) {
            let headerSpan = document.createElement("span");
            headerSpan.innerHTML = `<i class='fa fa-info-circle'></i> ${t("version.available_title")}`;

            let bodyDiv = document.createElement("div");
            bodyDiv.innerHTML = `${t("version.available_body")} <a href="https://libreqos.com">${t("version.download_now")}</a>`;

            createBootstrapToast("versionToast", headerSpan, bodyDiv);
        }
    }, 100);
}
