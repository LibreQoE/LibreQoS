use std::sync::atomic::Ordering;

use ratatui::{
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Widget},
};

/// Used to display the CPU usage and RAM usage
pub fn cpu_display() -> impl Widget {
    use crate::bus::cpu_ram::*;
    let num_cpus = NUM_CPUS.load(Ordering::Relaxed);
    let cpu_usage = CPU_USAGE
        .iter()
        .take(num_cpus)
        .map(|x| x.load(Ordering::Relaxed))
        .collect::<Vec<_>>();
    let total_ram = TOTAL_RAM.load(Ordering::Relaxed);
    let used_ram = RAM_USED.load(Ordering::Relaxed);

    let ram_percent = 100.0 - ((used_ram as f64 / total_ram as f64) * 100.0);

    let ram_color = if ram_percent < 10.0 {
        Color::Red
    } else if ram_percent < 25.0 {
        Color::Yellow
    } else {
        Color::White
    };

    let mut span_buf = vec![
        Span::styled(" [ RAM: ", Style::default().fg(Color::Green)),
        Span::styled(
            format!("{:.0}% ", ram_percent),
            Style::default().fg(ram_color),
        ),
        Span::styled("CPU: ", Style::default().fg(Color::Green)),
    ];
    for cpu in cpu_usage {
        let color = if cpu < 10 {
            Color::White
        } else if cpu < 25 {
            Color::Yellow
        } else {
            Color::Red
        };
        span_buf.push(Span::styled(
            format!("{}% ", cpu),
            Style::default().fg(color),
        ));
    }
    span_buf.push(Span::styled(" ] ", Style::default().fg(Color::Green)));

    Block::new().borders(Borders::TOP).title(span_buf)
}
