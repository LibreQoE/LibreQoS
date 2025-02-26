mod cpu;
mod stats_ringbuffer;
mod table_helper;
pub use cpu::cpu_display;
mod network_sparkline;
pub use network_sparkline::*;
use ratatui::{Frame, layout::Rect};
pub mod help;
pub mod latency_histogram;
pub mod top_flows;
pub mod top_hosts;

pub enum MainWidget {
    Hosts,
    Flows,
}

pub trait TopWidget {
    /// When the widget is enabled, this is called to setup the link to the bus
    fn enable(&mut self);

    /// When the widget is disabled, this is called to allow the widget to cleanup
    fn disable(&mut self);

    /// Receive the allocated size for the widget from the layout system
    fn set_size(&mut self, size: Rect);

    /// Perform a tick to update the widget
    fn tick(&mut self);

    /// Render the widget
    fn render_to_frame(&mut self, frame: &mut Frame);
}
