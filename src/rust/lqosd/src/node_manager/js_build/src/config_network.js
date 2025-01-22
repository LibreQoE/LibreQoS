function start() {
    // Add save button handler
    $("#btnSaveNetwork").on('click', () => {
        alert("Save functionality coming soon!");
    });

    // Load network data
    $.get("/local-api/networkJson", (njs) => {
        network_json = njs;
        RenderNetworkJson();
    });
}

$(document).ready(start);
