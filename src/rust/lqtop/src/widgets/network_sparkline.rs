use super::{stats_ringbuffer::StatsRingBuffer, TopWidget};
use crate::bus::BusMessage;
use lqos_bus::BusResponse;
use lqos_utils::packet_scale::scale_bits;
use ratatui::{
    prelude::*,
    style::{Color, Style},
    symbols,
    widgets::*,
};
use std::sync::mpsc::{Receiver, Sender};

pub struct NetworkSparkline {
    bus_link: tokio::sync::mpsc::Sender<crate::bus::BusMessage>,
    rx: Receiver<BusResponse>,
    tx: Sender<BusResponse>,
    throughput: StatsRingBuffer<CurrentThroughput, 200>,
    current_throughput: CurrentThroughput,
    render_size: Rect,
}

impl TopWidget for NetworkSparkline {
    fn enable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::EnableTotalThroughput(
                self.tx.clone(),
            ))
            .unwrap();
    }

    fn disable(&mut self) {
        self.bus_link
            .blocking_send(crate::bus::BusMessage::DisableTotalThroughput)
            .unwrap();
    }

    fn set_size(&mut self, _size: Rect) {
        self.render_size = _size;
    }

    fn tick(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            if let BusResponse::CurrentThroughput {
                bits_per_second,
                packets_per_second,
                shaped_bits_per_second,
            } = msg
            {
                self.throughput.push(CurrentThroughput {
                    bits_per_second,
                    _packets_per_second: packets_per_second,
                    shaped_bits_per_second,
                });
                self.current_throughput = CurrentThroughput {
                    bits_per_second,
                    _packets_per_second: packets_per_second,
                    shaped_bits_per_second,
                };
            }
        }
    }

    fn render_to_frame(&mut self, frame: &mut Frame) {
        let mut raw_data = self.throughput.get_values_in_order();
        raw_data.reverse();

        let bps_down: Vec<(f64, f64)> = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val.bits_per_second.1 as f64))
            .collect();

        let bps_up: Vec<(f64, f64)> = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val.bits_per_second.0 as f64))
            .collect();

        let shaped_down: Vec<(f64, f64)> = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val.shaped_bits_per_second.1 as f64))
            .collect();

        let shaped_up: Vec<(f64, f64)> = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val.shaped_bits_per_second.0 as f64))
            .collect();

        let (up, down) = self.current_throughput.bits_per_second;
        let title = format!(
            " [Throughput (Down: {} Up: {})]",
            scale_bits(up),
            scale_bits(down)
        );

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Green));

        let datasets = vec![
            Dataset::default()
                .name("Throughput")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Cyan))
                .data(&bps_down),
            Dataset::default()
                .name("Throughput")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Cyan))
                .data(&bps_up),
            Dataset::default()
                .name("Shaped")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::LightGreen))
                .data(&shaped_down),
            Dataset::default()
                .name("Shaped")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::LightGreen))
                .data(&shaped_up),
        ];

        let bps_max = bps_down.iter().map(|(_, val)| *val).fold(0.0, f64::max);

        let bps_min = bps_up.iter().map(|(_, val)| *val).fold(0.0, f64::min);

        let shaped_max = shaped_down.iter().map(|(_, val)| *val).fold(0.0, f64::max);

        let shaped_min = shaped_up.iter().map(|(_, val)| *val).fold(0.0, f64::min);

        let max = f64::max(bps_max, shaped_max);
        let min = f64::min(bps_min, shaped_min);

        let chart = Chart::new(datasets)
            .block(block)
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, 80.0]),
            )
            .y_axis(
                Axis::default()
                    .style(Style::default().fg(Color::Gray))
                    .bounds([min, max]),
            );

        frame.render_widget(chart, self.render_size);
    }
}

impl NetworkSparkline {
    pub fn new(bus_link: tokio::sync::mpsc::Sender<BusMessage>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<BusResponse>();

        NetworkSparkline {
            bus_link,
            rx,
            tx,
            throughput: StatsRingBuffer::new(),
            render_size: Rect::default(),
            current_throughput: CurrentThroughput::default(),
        }
    }
}

#[derive(Default, Copy, Clone)]
struct CurrentThroughput {
    pub bits_per_second: (u64, u64),
    pub _packets_per_second: (u64, u64),
    pub shaped_bits_per_second: (u64, u64),
}
