//! Provides a basic system for the UI framework. Handles
//! rendering the basic layout, talking to the UI framework,
//! and event-loop events that aren't quitting the program.
//!
//! It's designed to be the manager from which specific UI
//! components are managed.

use crate::{bus::BusMessage, widgets::*};
use ratatui::prelude::*;
use tokio::sync::mpsc::Sender;
use std::io::Stdout;

pub struct TopUi {
    show_cpus: bool,
    show_throughput_sparkline: bool,
    _bus_sender: Sender<BusMessage>,
    sparkline: NetworkSparkline,
    main_widget: MainWidget,
}

impl TopUi {
    /// Create a new TopUi instance. This will initialize the UI framework.
    pub fn new(bus_sender: Sender<BusMessage>) -> Self {
        TopUi {
            show_cpus: true,
            show_throughput_sparkline: false,
            main_widget: MainWidget::Hosts,
            _bus_sender: bus_sender.clone(),
            sparkline: NetworkSparkline::new(bus_sender.clone()),
        }
    }

    pub fn handle_keypress(&mut self, key: char) {
        // Handle Mode Switches
        match key {
            'c' => self.show_cpus = !self.show_cpus,
            'n' => {
                self.show_throughput_sparkline = !self.show_throughput_sparkline;
                if self.show_throughput_sparkline {
                    self.sparkline.enable();
                } else {
                    self.sparkline.disable();
                }
            }
            'h' => self.main_widget = MainWidget::Hosts,
            'f' => self.main_widget = MainWidget::Flows,
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

    fn top_level_render(&mut self, frame: &mut Frame) {
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

        let final_region = constraints.len();
        constraints.push(Constraint::Fill(1));

        let main_layout = Layout::new(Direction::Vertical, constraints).split(frame.size());

        // Add Widgets
        if self.show_cpus {
            frame.render_widget(cpu_display(), main_layout[cpu_region]);
        }
        if self.show_throughput_sparkline {
            self.sparkline.set_size(main_layout[network_spark_region]);
            self.sparkline.tick();
            self.sparkline.render_to_frame(frame);
        }

        // And finally the main panel
        match self.main_widget {
            MainWidget::Hosts => {
                frame.render_widget(top_hosts::hosts(), main_layout[final_region]);
            }
            MainWidget::Flows => {
                frame.render_widget(top_flows::flows(), main_layout[final_region]);
            }
        }
    }
}
