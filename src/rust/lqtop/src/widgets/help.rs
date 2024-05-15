use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

fn keyhelp(key: char, action: &'static str, buf: &mut Vec<Span>) {
    buf.push(Span::styled("[", Style::default().fg(Color::Green)));
    buf.push(Span::styled(key.to_string(), Style::default().fg(Color::Green)));
    buf.push(Span::styled("] ", Style::default().fg(Color::Green)));
    buf.push(Span::styled(action, Style::default().fg(Color::Yellow)));
    buf.push(Span::styled(" ", Style::default().fg(Color::Green)));
}

pub fn help_display() -> impl Widget {
    let mut span_buf = vec![
        Span::styled("LQTOP - ", Style::default().fg(Color::White)),
    ];
    keyhelp('q', "Quit", &mut span_buf);
    keyhelp('c', "CPUs", &mut span_buf);
    keyhelp('n', "Network", &mut span_buf);
    keyhelp('h', "Hosts", &mut span_buf);
    keyhelp('f', "Flows", &mut span_buf);
    keyhelp('l', "Latency Histogram", &mut span_buf);
    Block::new().borders(Borders::NONE).title(span_buf)
}