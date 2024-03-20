mod cpu;
pub use cpu::cpu_display;
mod network_sparkline;
pub use network_sparkline::*;
use ratatui::widgets::Widget;

pub enum MainWidget {
    Hosts,
}

impl MainWidget {
    pub fn render(&self) -> impl Widget + '_ {
        ratatui::widgets::Block::new()
    }
}
