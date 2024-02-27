use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use once_cell::sync::Lazy;
use std::sync::Mutex;

pub static ALL_FLOWS: Lazy<Mutex<Vec<(FlowbeeKey, FlowbeeData)>>> =
    Lazy::new(|| Mutex::new(Vec::with_capacity(128_000)));

