use wasm_bindgen::prelude::*;
use wasm_pipe_types::WasmRequest;
mod conduit;
mod message;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    #[wasm_bindgen(js_name = "window.bus.getToken")]
    fn get_token() -> String;

    #[wasm_bindgen(js_name = "window.onAuthOk")]
    fn onAuthOk(token: String, name: String, license_key: String);

    #[wasm_bindgen(js_name = "window.onAuthFail")]
    fn onAuthFail();

    #[wasm_bindgen(js_name = "window.onMessage")]
    fn onMessage(json: String);
}

pub use conduit::{initialize_wss, is_wasm_connected, send_wss_queue};

fn send_message(msg: WasmRequest) {
    //log(&format!("Enqueueing message: {msg:?}"));
    conduit::send_message(msg);
}

#[wasm_bindgen]
pub fn send_token(token: String) {
    //log(&format!("Sending token: {token}"));
    if token.is_empty() {
        log("Token is empty");
        return;
    }
    send_message(WasmRequest::Auth { token });
}

#[wasm_bindgen]
pub fn send_login(license: String, username: String, password: String) {
    let msg = WasmRequest::Login { license, username, password };
    send_message(msg);
}

#[wasm_bindgen]
pub fn request_node_status() {
    send_message(WasmRequest::GetNodeStatus);
}

#[wasm_bindgen]
pub fn request_packet_chart(period: String) {
    send_message(WasmRequest::PacketChart { period });
}

#[wasm_bindgen]
pub fn request_packet_chart_for_node(period: String, node_id: String, node_name: String) {
    send_message(WasmRequest::PacketChartSingle { period, node_id, node_name });
}

#[wasm_bindgen]
pub fn request_throughput_chart(period: String) {
    send_message(WasmRequest::ThroughputChart  { period });
}

#[wasm_bindgen]
pub fn request_throughput_chart_for_site(period: String, site_id: String) {
    send_message(WasmRequest::ThroughputChartSite { period, site_id });
}

#[wasm_bindgen]
pub fn request_throughput_chart_for_node(period: String, node_id: String, node_name: String) {
    send_message(WasmRequest::ThroughputChartSingle { period, node_id, node_name });
}

#[wasm_bindgen]
pub fn request_throughput_chart_for_circuit(period: String, circuit_id: String) {
    send_message(WasmRequest::ThroughputChartCircuit { period, circuit_id });
}

#[wasm_bindgen]
pub fn request_site_stack(period: String, site_id: String) {
    send_message(WasmRequest::SiteStack { period, site_id });
}

#[wasm_bindgen]
pub fn request_rtt_chart(period: String) {
    send_message(WasmRequest::RttChart  { period });
}

#[wasm_bindgen]
pub fn request_rtt_histogram(period: String) {
    send_message(WasmRequest::RttHistogram { period });
}

#[wasm_bindgen]
pub fn request_rtt_chart_for_site(period: String, site_id: String) {
    send_message(WasmRequest::RttChartSite  { period, site_id });
}

#[wasm_bindgen]
pub fn request_rtt_chart_for_node(period: String, node_id: String, node_name: String) {
    send_message(WasmRequest::RttChartSingle { period, node_id, node_name });
}

#[wasm_bindgen]
pub fn request_rtt_chart_for_circuit(period: String, circuit_id: String) {
    send_message(WasmRequest::RttChartCircuit { period, circuit_id });
}

#[wasm_bindgen]
pub fn request_node_perf_chart(period: String, node_id: String, node_name: String) {
    send_message(WasmRequest::NodePerfChart { period, node_id, node_name });
}

#[wasm_bindgen]
pub fn request_root_heat(period: String) {
    send_message(WasmRequest::RootHeat { period });
}

#[wasm_bindgen]
pub fn request_site_heat(period: String, site_id: String) {
    send_message(WasmRequest::SiteHeat { period, site_id });
}

#[wasm_bindgen]
pub fn request_tree(parent: String) {
    send_message(WasmRequest::Tree { parent });
}

#[wasm_bindgen]
pub fn request_site_info(site_id: String) {
    send_message(WasmRequest::SiteInfo { site_id });
}

#[wasm_bindgen]
pub fn request_site_parents(site_id: String) {
    send_message(WasmRequest::SiteParents { site_id });
}

#[wasm_bindgen]
pub fn request_circuit_parents(circuit_id: String) {
    send_message(WasmRequest::CircuitParents { circuit_id });
}

#[wasm_bindgen]
pub fn request_root_parents() {
    send_message(WasmRequest::RootParents);
}

#[wasm_bindgen]
pub fn request_search(term: String) {
    send_message(WasmRequest::Search { term });
}

#[wasm_bindgen]
pub fn request_circuit_info(circuit_id: String) {
    send_message(WasmRequest::CircuitInfo { circuit_id });
}

#[wasm_bindgen]
pub fn request_ext_device_info(circuit_id: String) {
    send_message(WasmRequest::ExtendedDeviceInfo { circuit_id });
}

#[wasm_bindgen]
pub fn request_ext_snr_graph(period: String, device_id: String) {
    send_message(WasmRequest::SignalNoiseChartExt { period, device_id });
}

#[wasm_bindgen]
pub fn request_ext_capacity_graph(period: String, device_id: String) {
    send_message(WasmRequest::DeviceCapacityChartExt { period, device_id });
}

#[wasm_bindgen]
pub fn request_ext_capacity_ap(period: String, site_name: String) {
    send_message(WasmRequest::ApCapacityExt { period, site_name });
}

#[wasm_bindgen]
pub fn request_ext_signal_ap(period: String, site_name: String) {
    send_message(WasmRequest::ApSignalExt { period, site_name });
}