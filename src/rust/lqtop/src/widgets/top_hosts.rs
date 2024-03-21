use lqos_bus::{BusResponse, IpStats};
use lqos_utils::packet_scale::{scale_bits, scale_packets};
use ratatui::prelude::*;
use super::{table_helper::TableHelper, TopWidget};

pub struct TopHosts {
    bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>,
    rx: std::sync::mpsc::Receiver<BusResponse>,
    tx: std::sync::mpsc::Sender<BusResponse>,
    size: Rect,
    stats: Vec<IpStats>,
}

impl TopWidget for TopHosts {
    fn enable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::EnableTopHosts(self.tx.clone()))
            .unwrap();
    }

    fn disable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::DisableTopHosts)
            .unwrap();
    }

    fn set_size(&mut self, size: Rect) {
        self.size = size;
    }

    fn tick(&mut self) {
        while let Ok(response) = self.rx.try_recv() {
            if let BusResponse::TopDownloaders(stats) = response {
                self.stats = stats;
            }
        }
    }

    fn render_to_frame(&mut self, frame: &mut Frame) {
        let mut t = TableHelper::new([
            "IP Address",
            "Down (bps)",
            "Up (bps)",
            "Down (pps)",
            "Up (pps)",
            "RTT",
            "TC Handle",
        ]);

        for host in self.stats.iter() {
            t.add_row([
                host.ip_address.to_string(),
                scale_bits(host.bits_per_second.0),
                scale_bits(host.bits_per_second.1),
                scale_packets(host.packets_per_second.0),
                scale_packets(host.packets_per_second.1),
                format!("{:.2} ms", host.median_tcp_rtt),
                host.tc_handle.to_string(),
            ]);
        }

        let block = t.to_block();
        frame.render_widget(block, self.size);
    }
}

impl TopHosts {
    pub fn new(bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<BusResponse>();
        Self {
            bus_link,
            rx,
            tx,
            size: Rect::default(),
            stats: Vec::new(),
        }
    }
}
