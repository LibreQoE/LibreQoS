use super::{table_helper::TableHelper, TopWidget};
use lqos_bus::{BusResponse, FlowbeeSummaryData};
use lqos_utils::packet_scale::scale_bits;
use ratatui::prelude::*;

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
                flow.local_ip.to_string(),
                flow.remote_ip.to_string(),
                flow.analysis.to_string(),
                scale_bits(flow.bytes_sent[0]),
                scale_bits(flow.bytes_sent[1]),
                format!("{}/{}", flow.tcp_retransmits[0], flow.tcp_retransmits[1]),
                format!("{:.1}/{:.1}", flow.rtt_nanos[0] as f64 / 1000000., flow.tcp_retransmits[1] as f64 / 1000000.),
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
