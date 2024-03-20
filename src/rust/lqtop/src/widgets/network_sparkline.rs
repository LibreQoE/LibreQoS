use crate::bus::throughput::{CURRENT_THROUGHPUT, THROUGHPUT_RING};
use lqos_utils::packet_scale::scale_bits;
use ratatui::{
    style::{Color, Style},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, Widget},
};

pub struct NetworkSparkline {
    bps_down: Vec<(f64, f64)>,
    bps_up: Vec<(f64, f64)>,
    shaped_down: Vec<(f64, f64)>,
    shaped_up: Vec<(f64, f64)>,
}

impl NetworkSparkline {
    pub fn new() -> Self {
        let raw_data = THROUGHPUT_RING.lock().unwrap().bits_per_second_vec_down();
        let bps_down = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val as f64))
            .collect();

        let raw_data = THROUGHPUT_RING.lock().unwrap().bits_per_second_vec_up();
        let bps_up = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, 0.0 - val as f64))
            .collect();

        let raw_data = THROUGHPUT_RING
            .lock()
            .unwrap()
            .shaped_bits_per_second_vec_down();
        let shaped_down = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, val as f64))
            .collect();

        let raw_data = THROUGHPUT_RING
            .lock()
            .unwrap()
            .shaped_bits_per_second_vec_up();
        let shaped_up = raw_data
            .iter()
            .enumerate()
            .map(|(i, &val)| (i as f64, 0.0 - val as f64))
            .collect();

        NetworkSparkline {
            bps_down,
            bps_up,
            shaped_down,
            shaped_up,
        }
    }

    pub fn render(&self) -> impl Widget + '_ {
        let (up, down) = CURRENT_THROUGHPUT.lock().unwrap().bits_per_second;
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
                .data(&self.bps_down),
            Dataset::default()
                .name("Throughput")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Cyan))
                .data(&self.bps_up),
            Dataset::default()
                .name("Shaped")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::LightGreen))
                .data(&self.shaped_down),
            Dataset::default()
                .name("Shaped")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::LightGreen))
                .data(&self.shaped_up),
        ];

        let bps_max = self
            .bps_down
            .iter()
            .map(|(_, val)| *val)
            .fold(0.0, f64::max);

        let bps_min = self.bps_up.iter().map(|(_, val)| *val).fold(0.0, f64::min);

        let shaped_max = self
            .shaped_down
            .iter()
            .map(|(_, val)| *val)
            .fold(0.0, f64::max);

        let shaped_min = self
            .shaped_up
            .iter()
            .map(|(_, val)| *val)
            .fold(0.0, f64::min);

        let max = f64::max(bps_max, shaped_max);
        let min = f64::min(bps_min, shaped_min);

        Chart::new(datasets)
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
            )
    }
}
