//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use dashmap::DashMap;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use once_cell::sync::Lazy;
use super::flow_analysis::FlowAnalysis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AsnId(pub u32);

pub static ALL_FLOWS: Lazy<DashMap<FlowbeeKey, (FlowbeeData, FlowAnalysis)>> = Lazy::new(|| DashMap::new());

