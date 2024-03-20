//! Provides a basic system for the UI framework. Handles
//! rendering the basic layout, talking to the UI framework,
//! and event-loop events that aren't quitting the program.
//!
//! It's designed to be the manager from which specific UI
//! components are managed.

use crate::{bus::BusCommand, widgets::*};
use ratatui::prelude::*;
use std::io::Stdout;
use tokio::sync::mpsc::Sender;

pub struct TopUi {
    show_cpus: bool,
    show_throughput_sparkline: bool,
    main_widget: MainWidget,
}

impl TopUi {
    /// Create a new TopUi instance. This will initialize the UI framework.
    pub fn new() -> Self {
        TopUi {
            show_cpus: true,
            show_throughput_sparkline: true,
            main_widget: MainWidget::Hosts,
        }
    }

    pub fn handle_keypress(&mut self, key: char, commander: Sender<BusCommand>) {
        // Handle Mode Switches
        match key {
            'c' => self.show_cpus = !self.show_cpus,
            'n' => {
                self.show_throughput_sparkline = !self.show_throughput_sparkline;
                commander.blocking_send(BusCommand::CollectTotalThroughput(
                    self.show_throughput_sparkline,
                )).unwrap();
            }
            _ => {}
        }
    }

    pub fn render(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
        terminal
            .draw(|f| {
                self.top_level_render(f);
            })
            .unwrap();
    }

    fn top_level_render(&self, frame: &mut Frame) {
        let mut constraints = Vec::new();
        let mut next_region = 0;

        // Build the layout regions
        let cpu_region = if self.show_cpus {
            constraints.push(Constraint::Length(1));
            next_region += 1;
            next_region - 1
        } else {
            next_region
        };

        let network_spark_region = if self.show_throughput_sparkline {
            constraints.push(Constraint::Length(10));
            next_region += 1;
            next_region - 1
        } else {
            next_region
        };

        // With a minimum of 1 row, we can now build the layout
        if constraints.is_empty() {
            constraints.push(Constraint::Min(1));
        }
        constraints.push(Constraint::Fill(1));

        let main_layout = Layout::new(Direction::Vertical, constraints).split(frame.size());

        // Add Widgets
        if self.show_cpus {
            frame.render_widget(cpu_display(), main_layout[cpu_region]);
        }
        if self.show_throughput_sparkline {
            let nspark = NetworkSparkline::new();
            let render = nspark.render();
            frame.render_widget(render, main_layout[network_spark_region]);
        }

        // And finally the main panel
        frame.render_widget(self.main_widget.render(), main_layout[next_region]);
    }
}
