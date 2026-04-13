use super::{TopWidget, table_helper::TableHelper};
use lqos_bus::{BusResponse, FlowbeeSummaryData};
use lqos_utils::packet_scale::scale_bits;
use ratatui::prelude::*;

fn truncate_by_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn flow_circuit_label(flow: &FlowbeeSummaryData) -> String {
    let candidate = if !flow.circuit_name.trim().is_empty() {
        flow.circuit_name.trim()
    } else {
        flow.circuit_id.trim()
    };
    if candidate.is_empty() {
        "-".to_string()
    } else {
        truncate_by_chars(candidate, 28)
    }
}

pub struct TopFlows {
    bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>,
    rx: std::sync::mpsc::Receiver<BusResponse>,
    tx: std::sync::mpsc::Sender<BusResponse>,
    size: Rect,
    flows: Vec<FlowbeeSummaryData>,
}

impl TopWidget for TopFlows {
    fn enable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::EnableTopFlows(self.tx.clone()))
            .unwrap();
    }

    fn disable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::DisableTopFlows)
            .unwrap();
    }

    fn set_size(&mut self, size: Rect) {
        self.size = size;
    }

    fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if let BusResponse::TopFlows(flows) = msg {
                self.flows = flows;
            }
        }
    }

    fn render_to_frame(&mut self, frame: &mut Frame) {
        let mut t = TableHelper::new([
            "Circuit",
            "Src IP",
            "Dst IP",
            "Type",
            "Upload",
            "Download",
            "Retransmits",
            "RTT (ms)",
            "ASN",
        ]);
        for flow in self.flows.iter() {
            t.add_row([
                flow_circuit_label(flow),
                flow.local_ip.to_string(),
                flow.remote_ip.to_string(),
                flow.analysis.to_string(),
                scale_bits(flow.bytes_sent.down),
                scale_bits(flow.bytes_sent.up),
                format!("{}/{}", flow.tcp_retransmits.down, flow.tcp_retransmits.up),
                format!(
                    "{:.1}/{:.1}",
                    flow.rtt_nanos.down as f64 / 1000000.,
                    flow.tcp_retransmits.up as f64 / 1000000.
                ),
                flow.remote_asn_name.to_string(),
            ]);
        }
        let table = t.to_block();
        frame.render_widget(table, self.size);
    }
}

impl TopFlows {
    pub fn new(bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<BusResponse>();
        Self {
            bus_link,
            tx,
            rx,
            size: Rect::default(),
            flows: Vec::new(),
        }
    }
}
