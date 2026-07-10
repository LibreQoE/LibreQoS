export function shapedDeviceIdFromElement(element) {
    return element?.getAttribute("data-device-id") || "";
}

export function shapedDeviceIdMatches(row, deviceId) {
    return String(row?.device_id ?? "") === String(deviceId ?? "");
}

export function shapedDeviceRowForId(rows, deviceId) {
    return rows.find((row) => shapedDeviceIdMatches(row, deviceId));
}

export function handleShapedDeviceActionClick(event, action) {
    event.preventDefault();
    const deviceId = shapedDeviceIdFromElement(event.currentTarget);
    if (deviceId.length > 0) {
        action(deviceId);
    }
}
