use super::TopWidget;
use lqos_bus::{BusResponse, FlowbeeSummaryData};
use lqos_utils::packet_scale::scale_bits;
use ratatui::{prelude::*, widgets::*};

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
        let block = Block::default()
            //.title("Top Downloaders")
            .borders(Borders::NONE)
            .style(Style::default().fg(Color::Green));

        let mut rows = Vec::new();

        for flow in self.flows.iter() {
            rows.push(
                Row::new(vec![
                    Cell::from(Text::from(flow.local_ip.to_string())),
                    Cell::from(Text::from(flow.remote_ip.to_string())),
                    Cell::from(Text::from(flow.analysis.to_string())),
                    Cell::from(Text::from(scale_bits(flow.bytes_sent[0]))),
                    Cell::from(Text::from(scale_bits(flow.bytes_sent[1]))),
                    Cell::from(Text::from(format!(
                        "{}/{}",
                        flow.tcp_retransmits[0], flow.tcp_retransmits[1]
                    ))),
                    Cell::from(Text::from(format!(
                        "{:.1}/{:.1}",
                        flow.rtt_nanos[0] as f64 / 1000000.,
                        flow.tcp_retransmits[1] as f64 / 1000000.
                    ))),
                    Cell::from(Text::from(flow.remote_asn_name.to_string())),
                ])
                .style(style::Style::default().fg(Color::White)),
            );
        }

        let header = Row::new(vec![
            Cell::from(Text::from("Src IP")),
            Cell::from(Text::from("Dst IP")),
            Cell::from(Text::from("Type")),
            Cell::from(Text::from("Upload")),
            Cell::from(Text::from("Download")),
            Cell::from(Text::from("Retransmits")),
            Cell::from(Text::from("RTT (ms)")),
            Cell::from(Text::from("ASN")),
        ])
        .style(style::Style::default().fg(Color::Yellow).bg(Color::Blue));

        let table = Table::new(rows, [15, 15, 20, 14, 14, 10, 15, 20])
            .block(block)
            .header(header);

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
