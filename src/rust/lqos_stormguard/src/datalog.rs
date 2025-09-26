use crate::config::StormguardConfig;
use allocative::Allocative;
use lqos_utils::unix_time::unix_now;
use std::io::Write;
use tracing::debug;

#[derive(Allocative)]
pub enum LogCommand {
    SpeedChange {
        site: String,
        download: u64,
        upload: u64,
        state: String,
    },
}

pub fn start_datalog(
    config: &StormguardConfig,
) -> anyhow::Result<std::sync::mpsc::Sender<LogCommand>> {
    // Initialize the datalog system
    let (tx, rx) = std::sync::mpsc::channel();
    let log_path = config.log_filename.clone();
    std::thread::Builder::new()
        .name("StormguardLogger".to_string())
        .spawn(move || {
            run_datalog(rx, log_path);
        })?;
    Ok(tx)
}

/// This thread will receive messages from the main thread and log them
fn run_datalog(rx: std::sync::mpsc::Receiver<LogCommand>, path: Option<String>) {
    let Some(path) = &path else {
        // If no path is provided, exit the thread
        debug!("No log path provided, exiting datalog thread.");
        return;
    };

    // If the log file exists, delete it
    if std::path::Path::new(path).exists() {
        if let Err(e) = std::fs::remove_file(path) {
            eprintln!("Failed to delete existing log file: {}", e);
        }
    }

    // Create the log file if it doesn't exist with the header
    if let Err(e) = std::fs::File::create(path) {
        eprintln!("Failed to create log file: {}", e);
    } else {
        // Write the header to the file
        if let Err(e) = std::fs::write(
            path,
            "Time,Site,Download,Upload,DirectionChanged,CanIncrease,CanDecrease,SaturationMax,SaturationCurrent,RetransmitState,RttState\n",
        ) {
            eprintln!("Failed to write header to log file: {}", e);
        }
    }

    loop {
        match rx.recv() {
            Ok(message) => {
                // Open for append
                let mut file = match std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                {
                    Ok(file) => file,
                    Err(e) => {
                        eprintln!("Failed to open log file: {}", e);
                        continue;
                    }
                };
                // Write the message to the file
                match message {
                    LogCommand::SpeedChange {
                        site,
                        download,
                        upload,
                        state,
                    } => {
                        // Append the line to the file
                        let Ok(date_time) = unix_now() else {
                            eprintln!("Failed to get current time");
                            continue;
                        };
                        if let Err(e) = writeln!(
                            file,
                            "{},{},{},{},{}",
                            date_time, site, download, upload, state
                        ) {
                            eprintln!("Failed to write to log file: {}", e);
                        }
                    }
                }
            }
            Err(_) => {
                // If the channel is closed, exit the loop
                break;
            }
        }
    }
}
