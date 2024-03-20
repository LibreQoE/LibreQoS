mod cpu;
pub use cpu::cpu_display;
mod network_sparkline;
pub use network_sparkline::*;
pub mod top_hosts;

pub enum MainWidget {
    Hosts,
}
