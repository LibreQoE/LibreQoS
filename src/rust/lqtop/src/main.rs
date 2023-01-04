use anyhow::Result;
use crossterm::{event::KeyCode, terminal::enable_raw_mode};
use lqos_bus::{
    decode_response, encode_request, BusRequest, BusResponse, BusSession, IpStats, BUS_BIND_ADDRESS,
};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table},
    Terminal,
};

struct DataResult {
    totals: (u64, u64, u64, u64),
    top: Vec<IpStats>,
}

async fn get_data(n_rows: u16) -> Result<DataResult> {
    let mut result = DataResult {
        totals: (0, 0, 0, 0),
        top: Vec::new(),
    };
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await?;
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![
            BusRequest::GetCurrentThroughput,
            BusRequest::GetTopNDownloaders(n_rows as u32),
        ],
    };
    let msg = encode_request(&test)?;
    stream.write(&msg).await?;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf)?;

    for r in reply.responses.iter() {
        match r {
            BusResponse::CurrentThroughput {
                bits_per_second,
                packets_per_second,
                shaped_bits_per_second: _,
            } => {
                let tuple = (
                    bits_per_second.0,
                    bits_per_second.1,
                    packets_per_second.0,
                    packets_per_second.1,
                );
                result.totals = tuple;
            }
            BusResponse::TopDownloaders(top) => {
                result.top = top.clone();
            }
            _ => {}
        }
    }

    Ok(result)
}

fn draw_menu<'a>() -> Paragraph<'a> {
    let text = Spans::from(vec![
        Span::styled("Q", Style::default().fg(Color::Green)),
        Span::from("uit"),
    ]);

    Paragraph::new(text)
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White))
                .border_type(BorderType::Plain)
                .title("LibreQoS Monitor"),
        )
}

fn scale_packets(n: u64) -> String {
    if n > 1_000_000_000 {
        format!("{:.2} gpps", n as f32 / 1_000_000_000.0)
    } else if n > 1_000_000 {
        format!("{:.2} mpps", n as f32 / 1_000_000.0)
    } else if n > 1_000 {
        format!("{:.2} kpps", n as f32 / 1_000.0)
    } else {
        format!("{n} pps")
    }
}

fn scale_bits(n: u64) -> String {
    if n > 1_000_000_000 {
        format!("{:.2} gbit/s", n as f32 / 1_000_000_000.0)
    } else if n > 1_000_000 {
        format!("{:.2} mbit/s", n as f32 / 1_000_000.0)
    } else if n > 1_000 {
        format!("{:.2} kbit/s", n as f32 / 1_000.0)
    } else {
        format!("{n} bit/s")
    }
}

fn draw_pps<'a>(packets_per_second: (u64, u64), bits_per_second: (u64, u64)) -> Spans<'a> {
    let text = Spans::from(vec![
        Span::styled("🠗 ", Style::default().fg(Color::Yellow)),
        Span::from(scale_packets(packets_per_second.0)),
        Span::from(" "),
        Span::from(scale_bits(bits_per_second.0)),
        Span::styled(" 🠕 ", Style::default().fg(Color::Yellow)),
        Span::from(scale_packets(packets_per_second.1)),
        Span::from(" "),
        Span::from(scale_bits(bits_per_second.1)),
    ]);
    text
}

fn draw_top_pane<'a>(
    top: &[IpStats],
    packets_per_second: (u64, u64),
    bits_per_second: (u64, u64),
) -> Table<'a> {
    let rows: Vec<Row> = top
        .iter()
        .map(|stats| {
            let color = if stats.bits_per_second.0 < 500 {
                Color::DarkGray
            } else if stats.tc_handle.as_u32() == 0 {
                Color::Cyan
            } else {
                Color::LightGreen
            };
            Row::new(vec![
                Cell::from(stats.ip_address.clone()),
                Cell::from(format!("🠗 {}", scale_bits(stats.bits_per_second.0))),
                Cell::from(format!("🠕 {}", scale_bits(stats.bits_per_second.1))),
                Cell::from(format!("🠗 {}", scale_packets(stats.packets_per_second.0))),
                Cell::from(format!("🠕 {}", scale_packets(stats.packets_per_second.1))),
                Cell::from(format!("{:.2} ms", stats.median_tcp_rtt)),
                Cell::from(stats.tc_handle.to_string()),
            ]).style(Style::default().fg(color))
        })
        .collect();

    let header = Row::new(vec![
        "Local IP",
        "Download",
        "Upload",
        "Pkts Dn",
        "Pkts Up",
        "TCP RTT ms",
        "Shaper"
    ])
    .style(Style::default().fg(Color::Yellow));

    Table::new(rows)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(draw_pps(packets_per_second, bits_per_second)),
        )
        .widths(&[
            Constraint::Min(40),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(11),
            Constraint::Length(7),
        ])
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
    let mut packets = (0, 0);
    let mut bits = (0, 0);
    let mut top = Vec::new();
    // Initialize TUI
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut n_rows = 10;

    loop {
        if let Ok(result) = get_data(n_rows).await {
            let (bits_down, bits_up, packets_down, packets_up) = result.totals;
            packets = (packets_down, packets_up);
            bits = (bits_down, bits_up);
            top = result.top;
        }

        //terminal.clear()?;
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Min(3),
                        Constraint::Percentage(80),
                        Constraint::Percentage(10),
                    ]
                    .as_ref(),
                )
                .split(f.size());
            f.render_widget(draw_menu(), chunks[0]);
            n_rows = chunks[1].height;
            f.render_widget(draw_top_pane(&top, packets, bits), chunks[1]);
            //f.render_widget(bandwidth_chart(datasets.clone(), packets, bits, min, max), chunks[1]);
        })?;

        if crossterm::event::poll(Duration::from_secs(1)).unwrap() {
            if let crossterm::event::Event::Key(key) = crossterm::event::read().unwrap() {
                if key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    // Undo the crossterm stuff
    terminal.clear()?;
    terminal.show_cursor()?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}
