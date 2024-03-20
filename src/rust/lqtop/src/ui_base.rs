//! Provides a basic system for the UI framework.
//! Upon starting the program, it performs basic initialization.
//! It tracks "drop", so when the program exits, it can perform cleanup.

use crate::{bus::BusCommand, top_level_ui::TopUi};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::stdout,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{sync::mpsc::Sender, task::yield_now};

pub static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

pub struct UiBase {
    ui: TopUi,
    bus_commander: Sender<BusCommand>,
}

impl UiBase {
    /// Create a new UiBase instance. This will initialize the UI framework.
    pub fn new(bus_commander: Sender<BusCommand>) -> Result<Self> {
        // Crossterm mode setup
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;

        // Setup Control-C Handler for graceful shutdown
        ctrlc::set_handler(move || {
            Self::cleanup();
            std::process::exit(0);
        })
        .unwrap();

        // Return
        Ok(UiBase {
            ui: TopUi::new(),
            bus_commander,
        })
    }

    pub fn quit_program(&self) {
        self.bus_commander.blocking_send(BusCommand::Quit).unwrap();
        SHOULD_EXIT.store(true, Ordering::Relaxed);
    }

    /// Set the should_exit flag to true, which will cause the event loop to exit.
    pub async fn event_loop(&mut self) -> Result<()> {
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        while !SHOULD_EXIT.load(Ordering::Relaxed) {
            if event::poll(std::time::Duration::from_millis(50))? {
                // Retrieve the keypress information
                if let Event::Key(key) = event::read()? {
                    // Key press (down) event
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            // Quit the program
                            KeyCode::Char('q') => {
                                self.quit_program();
                            }
                            _ => {
                                let char: Option<char> = match key.code {
                                    KeyCode::Char(c) => Some(c),
                                    _ => None,
                                };
                                if let Some(c) = char {
                                    self.ui.handle_keypress(c, self.bus_commander.clone());
                                }
                            }
                        }
                    }
                }
            }

            // Perform rendering
            self.ui.render(&mut terminal);

            // Ensure that all the event handlers can fire
            yield_now().await;
        }
        Ok(())
    }

    fn cleanup() {
        disable_raw_mode().unwrap();
        stdout()
            .execute(crossterm::terminal::LeaveAlternateScreen)
            .unwrap();
    }
}

impl Drop for UiBase {
    fn drop(&mut self) {
        Self::cleanup();
    }
}
