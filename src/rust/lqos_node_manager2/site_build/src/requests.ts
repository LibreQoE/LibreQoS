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

export function requestNetworkTreeSummary() {
    window.bus.send({
        "type" : "networkTreeSummary"
    })
}

export function requestTop10Downloaders() {
    window.bus.send({
        "type" : "top10Downloaders"
    })
}