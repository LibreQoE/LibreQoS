export function loadConfig(onComplete) {
    $.get("/local-api/getConfig", (data) => {
        window.config = data;
        onComplete();
    });
}

export function saveConfig(onComplete) {
    $.ajax({
        type: "POST",
        url: "/local-api/updateConfig",
        data: JSON.stringify(window.config),
        contentType: 'application/json',
        success: () => {
            onComplete();
        },
        error: () => {
            alert("That didn't work");
        }
    });
}

export function saveNetworkAndDevices(network_json, shaped_devices, onComplete) {
    // Validate network_json structure
    if (!network_json || typeof network_json !== 'object') {
        alert("Invalid network configuration");
        return;
    }

    // Validate shaped_devices structure
    if (!Array.isArray(shaped_devices)) {
        alert("Invalid shaped devices configuration");
        return;
    }

    // Validate individual shaped devices
    const validationErrors = [];
    const validNodes = Object.keys(network_json);
    
    shaped_devices.forEach((device, index) => {
        // Required fields
        if (!device.circuit_id || device.circuit_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Circuit ID is required`);
        }
        if (!device.device_id || device.device_id.trim() === "") {
            validationErrors.push(`Device ${index + 1}: Device ID is required`);
        }

        // Parent node validation
        if (device.parent_node && validNodes.length > 0 && !validNodes.includes(device.parent_node)) {
            validationErrors.push(`Device ${index + 1}: Parent node '${device.parent_node}' does not exist`);
        }

        // Bandwidth validation
        if (device.download_min_mbps < 1 || device.upload_min_mbps < 1 ||
            device.download_max_mbps < 1 || device.upload_max_mbps < 1) {
            validationErrors.push(`Device ${index + 1}: Bandwidth values must be greater than 0`);
        }
    });

    if (validationErrors.length > 0) {
        alert("Validation errors:\n" + validationErrors.join("\n"));
        return;
    }

    // Prepare data for submission
    const submission = {
        network_json,
        shaped_devices
    };

    // Send to server with enhanced error handling
    $.ajax({
        type: "POST",
        url: "/local-api/updateNetworkAndDevices",
        contentType: 'application/json',
        data: JSON.stringify(submission),
        dataType: 'json', // Expect JSON response
        success: (response) => {
            try {
                if (response && response.success) {
                    if (onComplete) onComplete(true, "Saved successfully");
                } else {
                    const msg = response?.message || "Unknown error occurred";
                    if (onComplete) onComplete(false, msg);
                    alert("Failed to save: " + msg);
                }
            } catch (e) {
                console.error("Error parsing response:", e);
                if (onComplete) onComplete(false, "Invalid server response");
                alert("Invalid server response format");
            }
        },
        error: (xhr) => {
            let errorMsg = "Request failed";
            try {
                if (xhr.responseText) {
                    const json = JSON.parse(xhr.responseText);
                    errorMsg = json.message || xhr.responseText;
                } else if (xhr.statusText) {
                    errorMsg = xhr.statusText;
                }
                console.error("AJAX Error:", {
                    status: xhr.status,
                    statusText: xhr.statusText,
                    response: xhr.responseText
                });
            } catch (e) {
                console.error("Error parsing error response:", e);
                errorMsg = "Unknown error occurred";
            }
            
            if (onComplete) onComplete(false, errorMsg);
            alert("Error saving configuration: " + errorMsg);
        }
    });
}
