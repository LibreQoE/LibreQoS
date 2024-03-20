use lqos_utils::packet_scale::{scale_bits, scale_packets};
use ratatui::{prelude::*, widgets::*};

use crate::bus::top_hosts::TOP_HOSTS;

pub fn hosts() -> impl Widget {
    let block = Block::default()
        //.title("Top Downloaders")
        .borders(Borders::NONE)
        .style(Style::default().fg(Color::Green));

    let mut rows = Vec::new();

    let lock = TOP_HOSTS.lock().unwrap();
    for host in lock.iter() {
        let color = if host.tc_handle.to_string() == "0:0" {
            Color::White
        } else {
            Color::LightGreen
        };

        let bg_color = if host.median_tcp_rtt > 150.0 {
            Color::Red
        } else if host.median_tcp_rtt > 100.0 {
            Color::Yellow
        } else {
            Color::Black
        };

        rows.push(
            Row::new(vec![
                Cell::from(Text::from(host.ip_address.to_string())),
                Cell::from(Text::from(scale_bits(host.bits_per_second.0))),
                Cell::from(Text::from(scale_bits(host.bits_per_second.1))),
                Cell::from(Text::from(scale_packets(host.packets_per_second.0))),
                Cell::from(Text::from(scale_packets(host.packets_per_second.1))),
                Cell::from(Text::from(format!("{:.2} ms", host.median_tcp_rtt))),
                Cell::from(Text::from(host.tc_handle.to_string())),
            ]).style(style::Style::default().fg(color).bg(bg_color)),
        );
    }

    let header = Row::new(vec![
        Cell::from(Text::from("IP Address")),
        Cell::from(Text::from("Down (bps)")),
        Cell::from(Text::from("Up (bps)")),
        Cell::from(Text::from("Down (pps)")),
        Cell::from(Text::from("Up (pps)")),
        Cell::from(Text::from("RTT")),
        Cell::from(Text::from("TC Handle")),
    ]).style(style::Style::default().fg(Color::Yellow).bg(Color::Blue));

    Table::new(rows, [20, 15, 15, 10, 10, 10, 10])
        .block(block)
        .header(header)
}
