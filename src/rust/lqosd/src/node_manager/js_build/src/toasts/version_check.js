export function checkForUpgrades(parentId) {
    if (window.newVersion) {
        let div = document.createElement("div");
        div.innerHTML = "<i class='fa fa-info-circle'></i> A New Version of LibreQoS is Available";
        div.classList.add("alert", "alert-success");
        let parent = document.getElementById(parentId);
        parent.appendChild(div);
    }
}