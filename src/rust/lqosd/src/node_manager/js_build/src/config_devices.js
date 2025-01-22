// Placeholder for shaped devices configuration functionality
// This will be implemented in a future update

function start() {
    // Load shaped devices data
    $.get("/local-api/allShapedDevices", (data) => {
        console.log("Loaded shaped devices:", data);
    });
}

$(document).ready(start);
