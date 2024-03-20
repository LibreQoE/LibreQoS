use lqos_utils::packet_scale::scale_bits;
use ratatui::{prelude::*, widgets::*};

use crate::bus::top_flows::TOP_FLOWS;

pub fn flows() -> impl Widget {
    let block = Block::default()
        //.title("Top Downloaders")
        .borders(Borders::NONE)
        .style(Style::default().fg(Color::Green));

    let mut rows = Vec::new();

    let lock = TOP_FLOWS.lock().unwrap();
    for flow in lock.iter() {

        rows.push(
            Row::new(vec![
                Cell::from(Text::from(flow.local_ip.to_string())),
                Cell::from(Text::from(flow.remote_ip.to_string())),
                Cell::from(Text::from(flow.analysis.to_string())),
                Cell::from(Text::from(scale_bits(flow.bytes_sent[0]))),
                Cell::from(Text::from(scale_bits(flow.bytes_sent[1]))),
                Cell::from(Text::from(format!("{}/{}", flow.tcp_retransmits[0], flow.tcp_retransmits[1]))),
                Cell::from(Text::from(format!("{:.1}/{:.1}", flow.rtt_nanos[0] as f64 / 1000000. , flow.tcp_retransmits[1] as f64 / 1000000.))),
                Cell::from(Text::from(flow.remote_asn_name.to_string())),
            ]).style(style::Style::default().fg(Color::White)),
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
    ]).style(style::Style::default().fg(Color::Yellow).bg(Color::Blue));

    Table::new(rows, [15, 15, 20, 14, 14, 10, 15, 20])
        .block(block)
        .header(header)
}