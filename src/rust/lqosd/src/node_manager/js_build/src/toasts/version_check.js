import {createBootstrapToast} from "../lq_js_common/helpers/toasts";

export function checkForUpgrades() {
    // Wait 1 second to ensure the newVersion variable has been set
    setTimeout(() => {
        if (window.newVersion) {
            let headerSpan = document.createElement("span");
            headerSpan.innerHTML = "<i class='fa fa-info-circle'></i> New Version Available";

            let bodyDiv = document.createElement("div");
            bodyDiv.innerHTML = "A new version of LibreQoS is available. Please update to the latest version to ensure you have the latest features and bug fixes. <a href='https://libreqos.com'>Download Now</a>";

            createBootstrapToast("versionToast", headerSpan, bodyDiv);
        }
    }, 100);
}