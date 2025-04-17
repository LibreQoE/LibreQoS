use crate::config::TornadoConfig;
use std::io::Write;
use lqos_utils::unix_time::unix_now;

pub enum LogCommand {
    SpeedChange {
        site: String,
        download: u64,
        upload: u64,
    },
}

pub fn start_datalog(
    config: &TornadoConfig,
) -> anyhow::Result<std::sync::mpsc::Sender<LogCommand>> {
    // Initialize the datalog system
    let (tx, rx) = std::sync::mpsc::channel();
    let log_path = config.log_filename.clone();
    std::thread::Builder::new()
        .name("TornadoLogger".to_string())
        .spawn(move || {
            run_datalog(rx, log_path);
        })?;
    Ok(tx)
}

fn run_datalog(rx: std::sync::mpsc::Receiver<LogCommand>, path: Option<String>) {
    // This thread will receive messages from the main thread and log them
    loop {
        match rx.recv() {
            Ok(message) => {
                let Some(path) = &path else {
                    // Silently ignore if no path is provided
                    continue;
                };
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
                    } => {
                        // Append the line to the file
                        let Ok(date_time) = unix_now() else {
                            eprintln!("Failed to get current time");
                            continue; 
                        };
                        if let Err(e) = writeln!(file, "{},{},{},{}\n", date_time, site, download, upload) {
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
