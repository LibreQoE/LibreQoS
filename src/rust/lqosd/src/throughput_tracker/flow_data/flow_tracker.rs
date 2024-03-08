//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use dashmap::DashMap;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use once_cell::sync::Lazy;

pub static ALL_FLOWS: Lazy<DashMap<FlowbeeKey, FlowbeeData>> = Lazy::new(|| DashMap::new());

