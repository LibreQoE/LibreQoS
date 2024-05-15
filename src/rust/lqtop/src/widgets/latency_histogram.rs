use super::{table_helper::TableHelper, TopWidget};
use lqos_bus::{BusResponse, FlowbeeSummaryData};
use lqos_utils::packet_scale::scale_bits;
use ratatui::prelude::*;

pub struct LatencyHistogram {
    bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>,
    rx: std::sync::mpsc::Receiver<BusResponse>,
    tx: std::sync::mpsc::Sender<BusResponse>,
    size: Rect,
    histogram: Vec<u32>,
}

impl TopWidget for LatencyHistogram {
    fn enable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::EnableLatencyHistogram(self.tx.clone()))
            .unwrap();
    }

    fn disable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::DisableLatencyHistogram)
            .unwrap();
    }

    fn set_size(&mut self, size: Rect) {
        self.size = size;
    }

    fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if let BusResponse::RttHistogram(histogram) = msg {
                self.histogram = histogram;
            }
        }
    }

    fn render_to_frame(&mut self, frame: &mut Frame) {
        let bars: Vec<(String, u64)> = self.histogram.iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), *v as u64))
            .collect();
        let bars_mangled: Vec<_> = bars.iter().map(|(s,n)| {
            (s.as_str(), *n)
        }).collect();
        let bar = ratatui::widgets::BarChart::default()
            .bar_width(5)
            .bar_gap(1)
            .data(&bars_mangled);
        frame.render_widget(bar, self.size);
    }
}

impl LatencyHistogram {
    pub fn new(bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<BusResponse>();
        Self {
            bus_link,
            tx,
            rx,
            size: Rect::default(),
            histogram: Vec::new(),
        }
    }
}
