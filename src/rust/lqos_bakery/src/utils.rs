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
        .args(["-f", "-batch", path.to_str().unwrap_or_default()])
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
        // Add line numbers to aid debugging with tc -batch's reported line numbers
        let mut numbered = String::new();
        for (i, line) in lines.lines().enumerate() {
            // tc -batch reports 1-based line numbers
            numbered.push_str(&format!("{:>4}: {}\n", i + 1, line));
        }
        let detailed = format!(
            "Command error for ({purpose}): {}\nCommands with line numbers:\n{}",
            error_str.trim(),
            numbered
        );
        // Log to console
        error!(detailed);

        // Also persist to /tmp for easier debugging of large batches
        let ts = current_timestamp();
        let path_ts = Path::new("/tmp").join(format!("lqos_bakery_failed_{}.txt", ts));
        if let Ok(mut f) = File::create(&path_ts) {
            let _ = f.write_all(detailed.as_bytes());
            let _ = f.flush();
            error!(
                "Bakery wrote numbered command failure to {}",
                path_ts.display()
            );
        } else {
            error!(
                "Bakery failed to write numbered command failure file: {}",
                path_ts.display()
            );
        }
        // Convenience: keep a rolling 'last error' file
        let path_last = Path::new("/tmp/lqos_bakery_last_error.txt");
        if let Ok(mut f) = File::create(&path_last) {
            let _ = f.write_all(detailed.as_bytes());
            let _ = f.flush();
        }
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
