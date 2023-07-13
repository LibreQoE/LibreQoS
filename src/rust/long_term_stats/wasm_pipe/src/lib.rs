use std::collections::VecDeque;

use wasm_bindgen::prelude::*;
use wasm_pipe_types::{WasmRequest, WasmResponse};
use web_sys::{BinaryType, ErrorEvent, MessageEvent, WebSocket};

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

static mut CONNECTED: bool = false;
static mut WS: Option<WebSocket> = None;
static mut QUEUE: VecDeque<Vec<u8>> = VecDeque::new();
static mut URL: String = String::new();

#[wasm_bindgen]
pub fn connect_wasm_pipe(url: String) {
    unsafe {
        if CONNECTED {
            log("Already connected");
            return;
        }
        if !url.is_empty() {
            URL = url.clone();
        }
        WS = Some(WebSocket::new(&url).unwrap());
        if let Some(ws) = &mut WS {
            ws.set_binary_type(BinaryType::Arraybuffer);

            ws.set_binary_type(BinaryType::Arraybuffer);
            let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
                log("Message Received");
                if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let array = js_sys::Uint8Array::new(&abuf);
                    //let len = array.byte_length() as usize;
                    let raw = array.to_vec();
                    let decompressed = miniz_oxide::inflate::decompress_to_vec(&raw).unwrap();
                    let msg: WasmResponse = serde_cbor::from_slice(&decompressed).unwrap();
                    //log(&format!("Message: {:?}", msg));

                    match msg {
                        WasmResponse::AuthOk { token, name, license_key } => {
                            onAuthOk(token, name, license_key);
                        }
                        WasmResponse::AuthFail => {
                            onAuthFail();
                        }
                        _ => {
                            let json = serde_json::to_string(&msg).unwrap();
                            onMessage(json);
                        }
                    }
                }
            });
            let onerror_callback = Closure::<dyn FnMut(_)>::new(move |e: ErrorEvent| {
                log(&format!("Error Received: {e:?}"));
                CONNECTED = false;
            });
            let onclose_callback = Closure::<dyn FnMut(_)>::new(move |_e: ErrorEvent| {
                log("Close Received");
                CONNECTED = false;
            });
            let onopen_callback = Closure::<dyn FnMut(_)>::new(move |_e: ErrorEvent| {
                log("Open Received");
                CONNECTED = true;
                let token = get_token();
                send_token(token);
            });
            ws.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
            ws.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
            ws.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
            ws.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
            // Prevent closures from recursing
            onopen_callback.forget();
            onclose_callback.forget();
            onerror_callback.forget();
            onmessage_callback.forget();
        }
    }
}

#[wasm_bindgen]
pub fn is_wasm_connected() -> bool {
    unsafe { CONNECTED && WS.is_some() }
}

#[wasm_bindgen]
pub fn send_wss_queue() {
    //log("Call to send queue");
    unsafe {
        // Bail out if there's nothing to do
        if QUEUE.is_empty() {
            //log("Queue is empty");
            return;
        }

        // Send queued messages
        if let Some(ws) = &mut WS {
            while let Some(msg) = QUEUE.pop_front() {
                log(&format!("Sending message: {msg:?}"));
                ws.send_with_u8_array(&msg).unwrap();
            }
        } else {
            log("No WebSocket connection");
            CONNECTED = false;
            connect_wasm_pipe(String::new());
        }
    }
}

fn build_message(msg: WasmRequest) -> Vec<u8> {
    let cbor = serde_cbor::to_vec(&msg).unwrap();
    miniz_oxide::deflate::compress_to_vec(&cbor, 8)
}

fn send_message(msg: WasmRequest) {
    log(&format!("Sending message: {msg:?}"));
    let msg = build_message(msg);
    unsafe {
        QUEUE.push_back(msg);
    }
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