export function requestFlowCount() {
    window.bus.send({
        "type" : "flowcount"
    })
}

export function requestShapedDeviceCount() {
    window.bus.send({
        "type" : "shapeddevicecount"
    })
}