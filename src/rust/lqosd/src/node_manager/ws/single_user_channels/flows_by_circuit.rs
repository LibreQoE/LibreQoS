use crate::node_manager::local_api::circuit_activity::{
    CircuitFlowSankeyRow, CircuitTopAsnsData, CircuitTopAsnsQuery, CircuitTrafficFlowsPage,
    CircuitTrafficFlowsQuery, circuit_flow_sankey_rows, circuit_top_asns_data,
    circuit_traffic_flows_page,
};

pub fn circuit_flow_sankey_result(circuit: &str) -> Vec<CircuitFlowSankeyRow> {
    circuit_flow_sankey_rows(circuit)
}

pub fn circuit_top_asns_result(query: &CircuitTopAsnsQuery) -> CircuitTopAsnsData {
    circuit_top_asns_data(query)
}

pub fn circuit_traffic_flows_result(query: &CircuitTrafficFlowsQuery) -> CircuitTrafficFlowsPage {
    circuit_traffic_flows_page(query)
}
