use parking_lot::Mutex;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use tracing::{error, info};

/// Get the current Unix timestamp in seconds
pub(crate) fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

static FILE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn execute_in_memory(command_buffer: &Vec<Vec<String>>, purpose: &str) {
    let lock = FILE_LOCK.lock();
    info!(
        "Bakery: Executing in-memory commands: {} lines, for {purpose}",
        command_buffer.len()
    );

    let path = Path::new("/tmp/lqos_bakery_commands.txt");
    let Some(lines) = write_command_file(path, command_buffer) else {
        error!("Failed to write commands to file for {purpose}");
        return;
    };

    let Ok(output) = std::process::Command::new("/sbin/tc")
        .args(&["-f", "-batch", path.to_str().unwrap_or_default()])
        .output()
    else {
        let message = format!("Failed to execute tc batch command for {purpose}.");
        error!(message);
        return;
    };

    let output_str = String::from_utf8_lossy(&output.stdout)
        .replace("Error: Exclusivity flag on, cannot modify.\n", "");
    if !output_str.is_empty() {
        error!("Command output for ({purpose}): {:?}", output_str.trim());
    }

    let error_str = String::from_utf8_lossy(&output.stderr);
    if !error_str.is_empty() {
        let message = format!(
            "Command error for ({purpose}): {:?}.Error: {error_str}\n{lines}",
            error_str.trim()
        );
        error!(message);
    }

    drop(lock); // Explicitly drop the lock to release it. This happens automatically at the end of the scope, but it's good to be explicit.
}

pub(crate) fn write_command_file(path: &Path, commands: &Vec<Vec<String>>) -> Option<String> {
    let mut lines = String::new();
    let Ok(file) = File::create(path) else {
        error!("Failed to create output file: {}", path.display());
        return None;
    };
    let mut f = BufWriter::new(file);
    for line in commands {
        for (idx, entry) in line.iter().enumerate() {
            lines.push_str(entry);
            if let Err(e) = f.write_all(entry.as_bytes()) {
                error!("Failed to write to output file: {}", e);
                return None;
            }
            if idx < line.len() - 1 {
                lines.push(' ');
                if let Err(e) = f.write_all(b" ") {
                    error!("Failed to write space to output file: {}", e);
                    return None;
                }
            }
        }
        let newline = "\n";
        lines.push_str(newline);
        if let Err(e) = f.write_all(newline.as_bytes()) {
            error!("Failed to write newline to output file: {}", e);
            return None;
        }
    }
    if let Err(e) = f.flush() {
        error!("Failed to flush output file: {}", e);
        return None;
    }
    Some(lines)
}
