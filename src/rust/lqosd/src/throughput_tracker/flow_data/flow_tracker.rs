//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use std::sync::Mutex;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use once_cell::sync::Lazy;

pub static ALL_FLOWS: Lazy<Mutex<Vec<(FlowbeeKey, FlowbeeData)>>> =
    Lazy::new(|| Mutex::new(Vec::with_capacity(128_000)));