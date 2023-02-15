use anyhow::Result;
use crossterm::{
  event::{read, Event, KeyCode, KeyEvent, KeyModifiers},
  terminal::enable_raw_mode,
};
use lqos_bus::{BusClient, BusRequest, BusResponse, IpStats};
use lqos_utils::packet_scale::{scale_bits, scale_packets};
use std::{io, time::Duration};
use tui::{
  backend::CrosstermBackend,
  layout::{Alignment, Constraint, Direction, Layout},
  style::{Color, Style},
  text::{Span, Spans},
  widgets::{Block, BorderType, Cell, Paragraph, Row, Table},
  Terminal,
};

struct DataResult {
  totals: (u64, u64, u64, u64),
  top: Vec<IpStats>,
}

async fn get_data(client: &mut BusClient, n_rows: u16) -> Result<DataResult> {
  let mut result = DataResult { totals: (0, 0, 0, 0), top: Vec::new() };
  let requests = vec![
    BusRequest::GetCurrentThroughput,
    BusRequest::GetTopNDownloaders { start: 0, end: n_rows as u32 },
  ];
  for r in client.request(requests).await? {
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

fn draw_menu<'a>(is_connected: bool) -> Paragraph<'a> {
  let mut text = Spans::from(vec![
    Span::styled("Q", Style::default().fg(Color::White)),
    Span::from("uit"),
  ]);

  if !is_connected {
    text
      .0
      .push(Span::styled(" NOT CONNECTED ", Style::default().fg(Color::Red)))
  } else {
    text
      .0
      .push(Span::styled("     CONNECTED ", Style::default().fg(Color::Green)))
  }

  let para = Paragraph::new(text)
    .style(Style::default().fg(Color::White))
    .alignment(Alignment::Center)
    .block(
      Block::default()
        .style(Style::default().fg(Color::Green))
        .border_type(BorderType::Plain)
        .title("LibreQoS Monitor: "),
    );

  para
}

fn draw_pps<'a>(
  packets_per_second: (u64, u64),
  bits_per_second: (u64, u64),
) -> Spans<'a> {
  Spans::from(vec![
    Span::from("DOWN: "),
    Span::from(scale_bits(bits_per_second.0)),
    Span::from(" "),
    Span::from(scale_bits(bits_per_second.1)),
    Span::from(" "),
    Span::from("UP: "),
    Span::from(scale_packets(packets_per_second.0)),
    Span::from(" "),
    Span::from(scale_packets(packets_per_second.1)),
  ])
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
        Cell::from(format!("{:<13}", scale_bits(stats.bits_per_second.0))),
        Cell::from(format!("{:<13}", scale_bits(stats.bits_per_second.1))),
        Cell::from(format!(
          "{:<13}",
          scale_packets(stats.packets_per_second.0)
        )),
        Cell::from(format!(
          "{:<13}",
          scale_packets(stats.packets_per_second.1)
        )),
        Cell::from(format!(
          "{:<10} ms",
          format!("{:.2}", stats.median_tcp_rtt)
        )),
        Cell::from(format!("{:>7}", stats.tc_handle.to_string())),
      ])
      .style(Style::default().fg(color))
    })
    .collect();

  let header = Row::new(vec![
    "Local IP",
    "Download",
    "Upload",
    "Pkts Dn",
    "Pkts Up",
    "TCP RTT ms",
    "Shaper",
  ])
  .style(Style::default().fg(Color::Yellow));

  Table::new(rows)
    .header(header)
    .block(
      Block::default().title(draw_pps(packets_per_second, bits_per_second)),
    )
    .widths(&[
      Constraint::Min(42),
      Constraint::Length(15),
      Constraint::Length(15),
      Constraint::Length(15),
      Constraint::Length(15),
      Constraint::Length(15),
      Constraint::Length(7),
    ])
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<()> {
  let mut bus_client = BusClient::new().await?;
  if !bus_client.is_connected() {
    println!("ERROR: lqosd bus is not available");
    std::process::exit(0);
  }
  let mut packets = (0, 0);
  let mut bits = (0, 0);
  let mut top = Vec::new();
  // Initialize TUI
  enable_raw_mode()?;
  let stdout = io::stdout();
  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;
  terminal.clear()?;
  let mut n_rows = 33;

  loop {
    if let Ok(result) = get_data(&mut bus_client, n_rows).await {
      let (bits_down, bits_up, packets_down, packets_up) = result.totals;
      packets = (packets_down, packets_up);
      bits = (bits_down, bits_up);
      top = result.top;
    }

    //terminal.clear()?;
    terminal.draw(|f| {
      let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(
          [Constraint::Min(1), Constraint::Percentage(100)].as_ref(),
        )
        .split(f.size());
      f.render_widget(draw_menu(bus_client.is_connected()), chunks[0]);
      // NOTE: this is where the height of the main panel is calculated.
      // Resize events are consumed by `tui`, so we never receive them.
      n_rows = chunks[1].height;
      f.render_widget(draw_top_pane(&top, packets, bits), chunks[1]);
      //f.render_widget(bandwidth_chart(datasets.clone(), packets, bits, min, max), chunks[1]);
    })?;

    if crossterm::event::poll(Duration::from_secs(1)).unwrap() {
      match read().unwrap() {
        // FIXME - this needs to absorb multiple resize events. Presently,
        // When I resize a terminal window, it is not getting one, either.
        // How to then change n_rows from here is also on my mind
        Event::Resize(width, height) => {
          println!("New size = {width}x{height}")
        }
        Event::Key(KeyEvent {
          code: KeyCode::Char('c'),
          modifiers: KeyModifiers::CONTROL,
          ..
        }) => break,
        Event::Key(KeyEvent {
          code: KeyCode::Char('q'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break,
        Event::Key(KeyEvent {
          code: KeyCode::Char('Z'),
          modifiers: KeyModifiers::CONTROL,
          ..
        }) => break, // Disconnect from bus, suspend
        //                   Event::Key(KeyEvent { escape should do something I don't know what.
        //                        code: KeyCode::Char('ESC'),
        //                        modifiers: KeyModifiers::CONTROL,}) => break,// go BACK?
        //
        Event::Key(KeyEvent {
          code: KeyCode::Char('h'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into help
        Event::Key(KeyEvent {
          code: KeyCode::Char('n'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into next
        // e.g. n_rows = screen size
        // n_start = n_start + screen
        // size
        Event::Key(KeyEvent {
          code: KeyCode::Char('p'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into prev
        Event::Key(KeyEvent {
          code: KeyCode::Char('?'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into help
        Event::Key(KeyEvent {
          code: KeyCode::Char('u'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into uploaders
        Event::Key(KeyEvent {
          code: KeyCode::Char('d'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into downloads
        Event::Key(KeyEvent {
          code: KeyCode::Char('c'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into cpu
        Event::Key(KeyEvent {
          code: KeyCode::Char('l'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME lag meter
        Event::Key(KeyEvent {
          code: KeyCode::Char('N'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into next panel
        Event::Key(KeyEvent {
          code: KeyCode::Char('P'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME make into prev panel
        Event::Key(KeyEvent {
          code: KeyCode::Char('b'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Best
        Event::Key(KeyEvent {
          code: KeyCode::Char('w'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Worst
        Event::Key(KeyEvent {
          code: KeyCode::Char('D'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Drops
        Event::Key(KeyEvent {
          code: KeyCode::Char('Q'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Queues
        Event::Key(KeyEvent {
          code: KeyCode::Char('W'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME (un)display wider stuff
        Event::Key(KeyEvent {
          code: KeyCode::Char('8'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Filter out fe80
        Event::Key(KeyEvent {
          code: KeyCode::Char('6'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Just look at ipv6
        Event::Key(KeyEvent {
          code: KeyCode::Char('4'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Just look at ipv4
        Event::Key(KeyEvent {
          code: KeyCode::Char('5'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME ipv4 + ipv6
        Event::Key(KeyEvent {
          code: KeyCode::Char('U'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME filter on Unshaped
        Event::Key(KeyEvent {
          code: KeyCode::Char('M'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME filter on My Network
        Event::Key(KeyEvent {
          code: KeyCode::Char('H'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Generate histogram
        Event::Key(KeyEvent {
          code: KeyCode::Char('T'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Filter Tin. This would require an argument BVIL<RET>
        Event::Key(KeyEvent {
          code: KeyCode::Char('O'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME "Odd" events - multicast, AI-assistance, people down?
        Event::Key(KeyEvent {
          code: KeyCode::Char('F'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Filter on "something*
        Event::Key(KeyEvent {
          code: KeyCode::Char('S'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Filter on Plan Speed
        Event::Key(KeyEvent {
          code: KeyCode::Char('z'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Zoom in
        Event::Key(KeyEvent {
          code: KeyCode::Char('Z'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Zoom out
        // Now I am Dreaming
        Event::Key(KeyEvent {
          code: KeyCode::Char('C'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Capture what I am filtering on
        Event::Key(KeyEvent {
          code: KeyCode::Char('F'),
          modifiers: KeyModifiers::CONTROL,
          ..
        }) => break, // FIXME Freeze what I am filtering on
        Event::Key(KeyEvent {
          code: KeyCode::Char('S'),
          modifiers: KeyModifiers::CONTROL,
          ..
        }) => break, // FIXME Step through what I captured on
        Event::Key(KeyEvent {
          code: KeyCode::Char('R'),
          modifiers: KeyModifiers::CONTROL,
          ..
        }) => break, // FIXME Step backwards what I captured on
        // Left and right cursors also
        // Dreaming Less now
        // Use TAB for autocompletion
        // If I have moved into a panel, the following are ideas
        Event::Key(KeyEvent {
          code: KeyCode::Char('/'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Search for ip
        Event::Key(KeyEvent {
          code: KeyCode::Char('R'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Traceroute/MTR
        Event::Key(KeyEvent {
          code: KeyCode::Char('A'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Alert me on this selection
        Event::Key(KeyEvent {
          code: KeyCode::Char('K'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME Kill Alert on this
        Event::Key(KeyEvent {
          code: KeyCode::Char('V'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // FIXME View Selected Alerts
        Event::Key(KeyEvent {
          code: KeyCode::Char('B'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // Launch Browser on this customer
        Event::Key(KeyEvent {
          code: KeyCode::Char('L'),
          modifiers: KeyModifiers::NONE,
          ..
        }) => break, // Log notebook on this set of filters
        _ => println!("Not recognized"),
      }
    }
  }

  // Undo the crossterm stuff
  terminal.clear()?;
  terminal.show_cursor()?;
  crossterm::terminal::disable_raw_mode()?;
  Ok(())
}
