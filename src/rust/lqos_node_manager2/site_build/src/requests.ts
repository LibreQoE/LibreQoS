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

export function requestThroughput() {
    window.bus.send({
        "type" : "throughput"
    })
}

export function requestFullThroughput() {
    window.bus.send({
        "type" : "throughputFull"
    })
}

export function requestRttHisto() {
    window.bus.send({
        "type" : "rttHisto"
    })
}