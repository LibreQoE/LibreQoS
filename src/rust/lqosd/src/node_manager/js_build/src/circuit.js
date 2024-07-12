// Obtain URL parameters
import {DirectChannel} from "./pubsub/direct_channels";

const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let circuit_id = decodeURI(params.id);
let channelLink = null;

function loadInitial() {
    $.ajax({
        type: "POST",
        url: "/local-api/circuitById",
        data: JSON.stringify({ id: circuit_id }),
        contentType: 'application/json',
        success: (circuits) => {
            console.log(circuits);
            let circuit = circuits[0];
            $("#circuitName").text(circuit.circuit_name);
            $("#parentNode").text(circuit.parent_node);
            $("#bwMax").text(circuit.download_max_mbps + " / " + circuit.upload_max_mbps + " Mbps");
            $("#bwMin").text(circuit.download_min_mbps + " / " + circuit.upload_min_mbps + " Mbps");

            channelLink = new DirectChannel({
                CircuitWatcher: {
                    circuit: circuit_id
                }
            }, (msg) => {
                //console.log(msg);
            });
        },
        error: () => {
            alert("Circuit with id " + circuit_id + " not found");
        }
    })
}

loadInitial();