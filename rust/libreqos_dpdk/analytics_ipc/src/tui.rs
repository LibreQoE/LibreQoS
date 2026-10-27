use crate::{
    AnalyticsSnapshot, DirectionalCounters, FlowId, FlowRow, HostId, LATENCY_BUCKETS,
    latency_histogram,
};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, BarChart, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table,
    TableState,
};
use std::collections::{HashMap, VecDeque};

const HISTORY_LEN: usize = 60;
type HistoryPoint = (Option<Rates>, Option<Rates>);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub download_bps: f64,
    pub upload_bps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Hosts,
    HostDetail,
    Flows,
}

pub struct App {
    pub snapshot: AnalyticsSnapshot,
    pub stale: bool,
    pub view: View,
    pub should_quit: bool,
    selected_host: Option<HostId>,
    table_state: TableState,
    previous: Option<AnalyticsSnapshot>,
    actual_rates: Option<Rates>,
    shaped_rates: Option<Rates>,
    host_rates: HashMap<HostId, Rates>,
    host_shaped_rates: HashMap<HostId, Rates>,
    host_retransmit: HashMap<HostId, Option<f32>>,
    flow_rates: HashMap<FlowId, Rates>,
    flow_retransmit: HashMap<FlowId, Option<f32>>,
    history: VecDeque<HistoryPoint>,
    host_history: HashMap<HostId, VecDeque<HistoryPoint>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            snapshot: AnalyticsSnapshot::default(),
            stale: true,
            view: View::Hosts,
            should_quit: false,
            selected_host: None,
            table_state: TableState::default(),
            previous: None,
            actual_rates: None,
            shaped_rates: None,
            host_rates: HashMap::new(),
            host_shaped_rates: HashMap::new(),
            host_retransmit: HashMap::new(),
            flow_rates: HashMap::new(),
            flow_retransmit: HashMap::new(),
            history: VecDeque::new(),
            host_history: HashMap::new(),
        }
    }
}

impl App {
    pub fn apply_snapshot(&mut self, mut snapshot: AnalyticsSnapshot) {
        snapshot.sort_and_limit(usize::MAX);
        let elapsed = self
            .previous
            .as_ref()
            .and_then(|old| {
                snapshot
                    .captured_at_millis
                    .checked_sub(old.captured_at_millis)
            })
            .filter(|elapsed| *elapsed > 0)
            .map(|elapsed| elapsed as f64 / 1_000.0);
        self.actual_rates = pair_rate(
            self.previous.as_ref().and_then(|old| old.actual.as_ref()),
            snapshot.actual.as_ref(),
            elapsed,
        );
        self.shaped_rates = pair_rate(
            self.previous.as_ref().and_then(|old| old.shaped.as_ref()),
            snapshot.shaped.as_ref(),
            elapsed,
        );
        self.host_rates.clear();
        self.host_shaped_rates.clear();
        self.host_retransmit.clear();
        self.flow_rates.clear();
        self.flow_retransmit.clear();
        if let (Some(old), Some(elapsed)) = (&self.previous, elapsed) {
            let old_hosts: HashMap<_, _> = old.hosts.iter().map(|host| (&host.id, host)).collect();
            for host in &snapshot.hosts {
                if let Some(old_host) = old_hosts.get(&host.id)
                    && let Some(rate) =
                        pair_rate(Some(&old_host.actual), Some(&host.actual), Some(elapsed))
                {
                    self.host_rates.insert(host.id.clone(), rate);
                }
                if let Some(old_host) = old_hosts.get(&host.id)
                    && let Some(rate) =
                        pair_rate(Some(&old_host.shaped), Some(&host.shaped), Some(elapsed))
                {
                    self.host_shaped_rates.insert(host.id.clone(), rate);
                }
                if let Some(old_host) = old_hosts.get(&host.id) {
                    self.host_retransmit.insert(
                        host.id.clone(),
                        retransmit_percent(
                            old_host.tcp_sent_packets,
                            old_host.tcp_retransmits,
                            host.tcp_sent_packets,
                            host.tcp_retransmits,
                        ),
                    );
                }
            }
            let old_flows: HashMap<_, _> = old
                .active_flows
                .iter()
                .chain(&old.recent_flows)
                .map(|flow| (&flow.id, flow))
                .collect();
            for flow in snapshot.active_flows.iter().chain(&snapshot.recent_flows) {
                if let Some(old_flow) = old_flows.get(&flow.id)
                    && let Some(rate) =
                        pair_rate(Some(&old_flow.actual), Some(&flow.actual), Some(elapsed))
                {
                    self.flow_rates.insert(flow.id.clone(), rate);
                }
                if let Some(old_flow) = old_flows.get(&flow.id) {
                    self.flow_retransmit.insert(
                        flow.id.clone(),
                        retransmit_percent(
                            old_flow.tcp_sent_packets,
                            old_flow.tcp_retransmits,
                            flow.tcp_sent_packets,
                            flow.tcp_retransmits,
                        ),
                    );
                }
            }
        }
        snapshot.hosts.sort_by(|a, b| {
            combined_rate(self.host_rates.get(&b.id))
                .total_cmp(&combined_rate(self.host_rates.get(&a.id)))
                .then_with(|| a.id.cmp(&b.id))
        });
        snapshot.active_flows.sort_by(|a, b| {
            combined_rate(self.flow_rates.get(&b.id))
                .total_cmp(&combined_rate(self.flow_rates.get(&a.id)))
                .then_with(|| a.id.cmp(&b.id))
        });
        snapshot.recent_flows.sort_by(|a, b| {
            combined_rate(self.flow_rates.get(&b.id))
                .total_cmp(&combined_rate(self.flow_rates.get(&a.id)))
                .then_with(|| a.id.cmp(&b.id))
        });
        self.history
            .push_back((self.actual_rates, self.shaped_rates));
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        let live_hosts: std::collections::HashSet<_> =
            snapshot.hosts.iter().map(|host| host.id.clone()).collect();
        self.host_history
            .retain(|host, _| live_hosts.contains(host));
        for host in &snapshot.hosts {
            let history = self.host_history.entry(host.id.clone()).or_default();
            history.push_back((
                self.host_rates.get(&host.id).copied(),
                self.host_shaped_rates.get(&host.id).copied(),
            ));
            while history.len() > HISTORY_LEN {
                history.pop_front();
            }
        }
        self.stale = false;
        self.previous = Some(snapshot.clone());
        self.snapshot = snapshot;
        self.reconcile_selection();
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub fn selected_host(&self) -> Option<&HostId> {
        self.selected_host.as_ref()
    }

    pub fn status_keys(&self) -> &'static str {
        match self.view {
            View::Hosts => "↑/↓ j/k navigate  Enter detail  f flows  q quit",
            View::HostDetail => "↑/↓ j/k navigate  Esc back  f flows  q quit",
            View::Flows => "↑/↓ j/k navigate  h hosts  q quit",
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') if self.view == View::Flows => {
                self.view = View::Hosts;
                self.reconcile_selection();
            }
            KeyCode::Char('f') if self.view != View::Flows => {
                self.view = View::Flows;
                self.table_state.select(Some(0));
            }
            KeyCode::Esc if self.view == View::HostDetail => {
                self.view = View::Hosts;
                self.reconcile_selection();
            }
            KeyCode::Enter if self.view == View::Hosts && self.selected_host.is_some() => {
                self.view = View::HostDetail;
                self.table_state.select(Some(0));
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            _ => {}
        }
    }

    fn visible_len(&self) -> usize {
        match self.view {
            View::Hosts => self.snapshot.hosts.len(),
            View::HostDetail => self.detail_flows().len(),
            View::Flows => self.snapshot.active_flows.len() + self.snapshot.recent_flows.len(),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible_len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(len - 1);
        self.table_state.select(Some(next));
        if self.view == View::Hosts {
            self.selected_host = self.snapshot.hosts.get(next).map(|host| host.id.clone());
        }
    }

    fn reconcile_selection(&mut self) {
        if self.view == View::Flows {
            let len = self.visible_len();
            self.table_state
                .select((len > 0).then_some(self.table_state.selected().unwrap_or(0).min(len - 1)));
            return;
        }
        let selected_host_index = self
            .selected_host
            .as_ref()
            .and_then(|id| self.snapshot.hosts.iter().position(|host| &host.id == id))
            .or((!self.snapshot.hosts.is_empty()).then_some(0));
        self.selected_host = selected_host_index.map(|index| self.snapshot.hosts[index].id.clone());
        if self.view == View::HostDetail && selected_host_index.is_none() {
            self.view = View::Hosts;
        }
        if self.view == View::HostDetail {
            let len = self.detail_flows().len();
            self.table_state
                .select((len > 0).then_some(self.table_state.selected().unwrap_or(0).min(len - 1)));
        } else {
            self.table_state.select(selected_host_index);
        }
    }

    fn detail_flows(&self) -> Vec<&FlowRow> {
        let Some(host) = &self.selected_host else {
            return Vec::new();
        };
        self.snapshot
            .active_flows
            .iter()
            .chain(&self.snapshot.recent_flows)
            .filter(|flow| {
                flow.id.circuit_id == host.circuit_id && flow.id.subscriber_ip == host.subscriber_ip
            })
            .collect()
    }
}

fn combined_rate(rate: Option<&Rates>) -> f64 {
    rate.map_or(0.0, |rate| rate.download_bps + rate.upload_bps)
}

fn pair_rate(
    previous: Option<&DirectionalCounters>,
    current: Option<&DirectionalCounters>,
    elapsed: Option<f64>,
) -> Option<Rates> {
    let (previous, current, elapsed) = (previous?, current?, elapsed?);
    Some(Rates {
        download_bps: current
            .download_bytes
            .saturating_sub(previous.download_bytes) as f64
            * 8.0
            / elapsed,
        upload_bps: current.upload_bytes.saturating_sub(previous.upload_bytes) as f64 * 8.0
            / elapsed,
    })
}

fn retransmit_percent(
    previous_sent: u64,
    previous_retransmits: u64,
    current_sent: u64,
    current_retransmits: u64,
) -> Option<f32> {
    let sent = current_sent.saturating_sub(previous_sent);
    if sent == 0 {
        return None;
    }
    let retransmits = current_retransmits.saturating_sub(previous_retransmits);
    Some(retransmits as f32 * 100.0 / sent as f32)
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(frame.area());
    render_header(frame, app, root[0]);
    match app.view {
        View::Hosts | View::HostDetail => render_hosts_layout(frame, app, root[1]),
        View::Flows => render_flows_table(frame, app, root[1], false),
    }
    let status = Line::from(vec![
        Span::styled("Keys: ", Style::default().fg(Color::Yellow)),
        Span::raw(app.status_keys()),
    ]);
    frame.render_widget(Paragraph::new(status), root[2]);
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let stale = if app.stale { "  STALE" } else { "" };
    let (actual, shaped) = if app.view == View::HostDetail {
        app.selected_host
            .as_ref()
            .map(|host| {
                (
                    app.host_rates.get(host).copied(),
                    app.host_shaped_rates.get(host).copied(),
                )
            })
            .unwrap_or_default()
    } else {
        (app.actual_rates, app.shaped_rates)
    };
    let text = format!(
        " lqtop{stale}  transmitted D {} U {}  shaped D {} U {} ",
        format_rate(actual.map(|rate| rate.download_bps)),
        format_rate(actual.map(|rate| rate.upload_bps)),
        format_rate(shaped.map(|rate| rate.download_bps)),
        format_rate(shaped.map(|rate| rate.upload_bps)),
    );
    let color = if app.stale { Color::Gray } else { Color::Blue };
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::White).bg(color)),
        area,
    );
}

fn render_hosts_layout(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);
    render_history(frame, app, vertical[0]);
    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(vertical[1]);
    if app.view == View::HostDetail {
        render_flows_table(frame, app, lower[0], true);
    } else {
        render_host_table(frame, app, lower[0]);
    }
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(lower[1]);
    let (rtt, dns) = selected_samples(app);
    render_histogram(frame, right[0], "RTT (10ms)", &rtt, Color::Cyan);
    render_histogram(frame, right[1], "DNS (10ms)", &dns, Color::LightGreen);
}

fn render_history(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let history = if app.view == View::HostDetail {
        app.selected_host
            .as_ref()
            .and_then(|host| app.host_history.get(host))
            .unwrap_or(&app.history)
    } else {
        &app.history
    };
    let series = |pick: fn(&HistoryPoint) -> Option<f64>| {
        history
            .iter()
            .enumerate()
            .filter_map(|(x, row)| pick(row).map(|y| (x as f64, y / 1_000_000.0)))
            .collect::<Vec<_>>()
    };
    let actual_down = series(|row| row.0.map(|rate| rate.download_bps));
    let actual_up = series(|row| row.0.map(|rate| rate.upload_bps));
    let shaped_down = series(|row| row.1.map(|rate| rate.download_bps));
    let shaped_up = series(|row| row.1.map(|rate| rate.upload_bps));
    let maximum = actual_down
        .iter()
        .chain(&actual_up)
        .chain(&shaped_down)
        .chain(&shaped_up)
        .map(|(_, value)| *value)
        .fold(1.0_f64, f64::max);
    let datasets = vec![
        Dataset::default()
            .name("actual D")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&actual_down),
        Dataset::default()
            .name("actual U")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::LightGreen))
            .data(&actual_up),
        Dataset::default()
            .name("shaped D")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Blue))
            .data(&shaped_down),
        Dataset::default()
            .name("shaped U")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Green))
            .data(&shaped_up),
    ];
    frame.render_widget(
        Chart::new(datasets)
            .block(block("60 second throughput (Mbit/s)", app.stale))
            .x_axis(Axis::default().bounds([0.0, 59.0]))
            .y_axis(Axis::default().bounds([0.0, maximum])),
        area,
    );
}

fn render_host_table(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let rows = app.snapshot.hosts.iter().map(|host| {
        let rate = app.host_rates.get(&host.id).copied().unwrap_or_default();
        let inactive = rate.download_bps == 0.0 && rate.upload_bps == 0.0;
        Row::new(vec![
            Cell::from(host.id.circuit_id.to_string()),
            Cell::from(host.id.subscriber_ip.to_string()),
            Cell::from(format_rate(Some(rate.download_bps))),
            Cell::from(format_rate(Some(rate.upload_bps))),
            Cell::from(median(&host.rtt_samples_us).map_or_else(|| "—".into(), format_latency)),
            Cell::from(format_percent(
                app.host_retransmit.get(&host.id).copied().flatten(),
            )),
        ])
        .style(if inactive || app.stale {
            Style::default().fg(Color::Gray)
        } else {
            Style::default()
        })
    });
    let title = format!("Hosts ({})", app.snapshot.hosts.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(9),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(["Circuit", "Subscriber", "D", "U", "RTT", "Retrans"])
            .style(Style::default().fg(Color::Blue)),
    )
    .block(block(&title, app.stale))
    .row_highlight_style(
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_flows_table(frame: &mut Frame<'_>, app: &mut App, area: Rect, host_only: bool) {
    let flows: Vec<&FlowRow> = if host_only {
        app.detail_flows()
    } else {
        app.snapshot
            .active_flows
            .iter()
            .chain(&app.snapshot.recent_flows)
            .collect()
    };
    let rows = flows.iter().map(|flow| {
        Row::new(vec![
            Cell::from(flow.id.circuit_id.to_string()),
            Cell::from(flow.id.subscriber_ip.to_string()),
            Cell::from(flow.id.remote_ip.to_string()),
            Cell::from(format!(
                "{}→{}",
                flow.id.subscriber_port, flow.id.remote_port
            )),
            Cell::from(format_rate(
                app.flow_rates.get(&flow.id).map(|rate| rate.download_bps),
            )),
            Cell::from(format_rate(
                app.flow_rates.get(&flow.id).map(|rate| rate.upload_bps),
            )),
            Cell::from(format_percent(
                app.flow_retransmit.get(&flow.id).copied().flatten(),
            )),
            Cell::from(format!("{:?}", flow.state)),
        ])
        .style(if app.stale || flow.state != crate::FlowState::Active {
            Style::default().fg(Color::Gray)
        } else {
            Style::default()
        })
    });
    let title = if host_only { "Host flows" } else { "Flows" };
    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Length(13),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(7),
        ],
    )
    .header(
        Row::new([
            "Circuit",
            "Subscriber",
            "Remote",
            "Ports",
            "D",
            "U",
            "Retrans",
            "State",
        ])
        .style(Style::default().fg(Color::Blue)),
    )
    .block(block(title, app.stale))
    .row_highlight_style(
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn selected_samples(app: &App) -> (Vec<u32>, Vec<u32>) {
    match &app.selected_host {
        Some(id) => app
            .snapshot
            .hosts
            .iter()
            .find(|host| &host.id == id)
            .map(|host| (host.rtt_samples_us.clone(), host.dns_samples_us.clone()))
            .unwrap_or_default(),
        None => (
            app.snapshot
                .hosts
                .iter()
                .flat_map(|host| host.rtt_samples_us.iter().copied())
                .collect(),
            app.snapshot
                .hosts
                .iter()
                .flat_map(|host| host.dns_samples_us.iter().copied())
                .collect(),
        ),
    }
}

fn render_histogram(frame: &mut Frame<'_>, area: Rect, title: &str, samples: &[u32], color: Color) {
    let bins = latency_histogram(samples);
    let available = area.width.saturating_sub(2) as usize;
    let group = LATENCY_BUCKETS.div_ceil(available.max(1)).max(1);
    let data: Vec<(String, u64)> = bins
        .chunks(group)
        .enumerate()
        .map(|(index, chunk)| {
            let start = index * group * 10;
            let label = if index * group + group >= LATENCY_BUCKETS {
                format!(">={start}")
            } else {
                start.to_string()
            };
            (label, chunk.iter().map(|value| u64::from(*value)).sum())
        })
        .collect();
    let refs: Vec<(&str, u64)> = data
        .iter()
        .map(|(label, value)| (label.as_str(), *value))
        .collect();
    frame.render_widget(
        BarChart::default()
            .block(block(title, false))
            .bar_width(1)
            .bar_gap(0)
            .bar_style(Style::default().fg(color))
            .data(&refs),
        area,
    );
}

fn block<'a>(title: &'a str, stale: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(Color::White)))
        .border_style(Style::default().fg(if stale { Color::Gray } else { Color::Green }))
}

fn format_rate(rate: Option<f64>) -> String {
    match rate {
        None => "—".into(),
        Some(rate) if rate >= 1_000_000_000.0 => format!("{:.1}G", rate / 1_000_000_000.0),
        Some(rate) if rate >= 1_000_000.0 => format!("{:.1}M", rate / 1_000_000.0),
        Some(rate) if rate >= 1_000.0 => format!("{:.1}K", rate / 1_000.0),
        Some(rate) => format!("{rate:.0}"),
    }
}

fn median(samples: &[u32]) -> Option<u32> {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    samples.get(samples.len() / 2).copied()
}

fn format_latency(micros: u32) -> String {
    format!("{:.1}ms", micros as f64 / 1_000.0)
}

fn format_percent(percent: Option<f32>) -> String {
    percent.map_or_else(|| "—".into(), |percent| format!("{percent:.1}%"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlowState, HostRow};
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};
    use std::net::{IpAddr, Ipv4Addr};

    fn host(octet: u8, bytes: u64) -> HostRow {
        HostRow {
            id: HostId {
                circuit_id: u64::from(octet),
                subscriber_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, octet)),
            },
            actual: DirectionalCounters {
                download_bytes: bytes,
                upload_bytes: bytes / 2,
            },
            shaped: DirectionalCounters {
                download_bytes: bytes / 2,
                upload_bytes: bytes / 4,
            },
            rtt_samples_us: vec![9_000, 11_000],
            dns_samples_us: vec![20_000],
            tcp_sent_packets: 100,
            tcp_retransmits: 2,
            last_seen_millis: 1_000,
            enqueued_packets: crate::DirectionalPackets {
                download_packets: bytes,
                upload_packets: bytes / 2,
            },
            sent_packets: crate::DirectionalPackets {
                download_packets: bytes / 2,
                upload_packets: bytes / 4,
            },
            ..HostRow::default()
        }
    }

    fn flow(octet: u8) -> FlowRow {
        FlowRow {
            id: FlowId {
                circuit_id: u64::from(octet),
                flow_hash: u64::from(octet),
                subscriber_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, octet)),
                remote_ip: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                subscriber_port: 1000,
                remote_port: 443,
                protocol: 6,
            },
            actual: DirectionalCounters {
                download_bytes: 100,
                upload_bytes: 20,
            },
            shaped: DirectionalCounters::default(),
            rtt_samples_us: vec![10_000],
            tcp_sent_packets: 100,
            tcp_retransmits: 2,
            first_seen_millis: 0,
            last_seen_millis: 1_000,
            state: FlowState::Active,
            ..FlowRow::default()
        }
    }

    fn snapshot(time: u64, hosts: Vec<HostRow>) -> AnalyticsSnapshot {
        AnalyticsSnapshot {
            captured_at_millis: time,
            actual: Some(DirectionalCounters {
                download_bytes: time * 100,
                upload_bytes: time * 50,
            }),
            shaped: Some(DirectionalCounters {
                download_bytes: time * 80,
                upload_bytes: time * 40,
            }),
            hosts,
            active_flows: vec![flow(1)],
            ..Default::default()
        }
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn layouts_header_and_dynamic_status_render() {
        let mut app = App::default();
        app.apply_snapshot(snapshot(1_000, vec![host(1, 100)]));
        app.apply_snapshot(snapshot(2_000, vec![host(1, 300)]));
        let hosts = screen(&mut app, 120, 35);
        assert!(hosts.contains("lqtop") && hosts.contains("transmitted D"));
        assert!(hosts.contains("Enter detail") && hosts.contains("60 second throughput"));
        app.handle_key(key(KeyCode::Enter));
        assert!(screen(&mut app, 120, 35).contains("Host flows"));
        assert!(app.status_keys().contains("Esc back"));
        app.handle_key(key(KeyCode::Char('f')));
        let flows = screen(&mut app, 120, 35);
        assert!(flows.contains("Flows") && app.status_keys().contains("h hosts"));
    }

    #[test]
    fn stable_selection_survives_sort_and_expiry_returns_to_hosts() {
        let mut app = App::default();
        app.apply_snapshot(snapshot(1_000, vec![host(1, 100), host(2, 200)]));
        app.handle_key(key(KeyCode::Down));
        let selected = app.selected_host().cloned().unwrap();
        app.apply_snapshot(snapshot(2_000, vec![host(1, 500), host(2, 600)]));
        assert_eq!(app.selected_host(), Some(&selected));
        app.handle_key(key(KeyCode::Enter));
        app.apply_snapshot(snapshot(3_000, vec![]));
        assert_eq!(app.view, View::Hosts);
    }

    #[test]
    fn scrolling_stale_and_narrow_histograms_render() {
        let mut app = App::default();
        app.apply_snapshot(snapshot(
            1_000,
            (1..20).map(|n| host(n, u64::from(n))).collect(),
        ));
        for _ in 0..12 {
            app.handle_key(key(KeyCode::Down));
        }
        app.mark_stale();
        let output = screen(&mut app, 52, 16);
        assert!(output.contains("STALE"));
        assert!(output.contains("RTT") || output.contains("DNS"));
    }

    #[test]
    fn unavailable_counters_render_as_unavailable() {
        let mut app = App::default();
        app.apply_snapshot(AnalyticsSnapshot {
            captured_at_millis: 1_000,
            hosts: vec![host(1, 0)],
            ..Default::default()
        });
        assert_eq!(app.actual_rates, None);
        assert_eq!(app.shaped_rates, None);
        assert!(screen(&mut app, 100, 20).contains('—'));
    }

    #[test]
    fn retransmit_percentage_uses_refresh_deltas_and_zero_is_unavailable() {
        let mut app = App::default();
        let mut first = host(1, 100);
        first.tcp_sent_packets = 100;
        first.tcp_retransmits = 4;
        app.apply_snapshot(snapshot(1_000, vec![first]));

        let mut second = host(1, 200);
        second.tcp_sent_packets = 120;
        second.tcp_retransmits = 5;
        app.apply_snapshot(snapshot(2_000, vec![second]));
        assert_eq!(
            app.host_retransmit.get(app.selected_host().unwrap()),
            Some(&Some(5.0))
        );

        let mut third = host(1, 300);
        third.tcp_sent_packets = 120;
        third.tcp_retransmits = 6;
        app.apply_snapshot(snapshot(3_000, vec![third]));
        assert_eq!(
            app.host_retransmit.get(app.selected_host().unwrap()),
            Some(&None)
        );
    }
}
