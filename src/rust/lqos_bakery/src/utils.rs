use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::Stdio;
use tracing::{error, info};

/// Get the current Unix timestamp in seconds
pub(crate) fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn execute_in_memory(command_buffer: &Vec<Vec<String>>, purpose: &str) {
    info!("Bakery: Executing in-memory commands: {} lines, for {purpose}", command_buffer.len());
    
    // Track TC commands executed
    crate::BAKERY_STATS.tc_commands_executed.fetch_add(command_buffer.len() as u64, std::sync::atomic::Ordering::Relaxed);

    /*for line in command_buffer {
        let Ok(output) = std::process::Command::new("/sbin/tc")
            .args(line)
            .output() else {
            error!("Failed to execute command: {:?}", line);
            continue;
        };
        //println!("/sbin/tc/{}", line.join(" "));
        let output_str = String::from_utf8_lossy(&output.stdout);
        if !output_str.is_empty() {
            error!("Executing command: ({purpose}) {:?}", line);
            error!("Command result: {:?}", output_str.trim());
        }
        let error_str = String::from_utf8_lossy(&output.stderr);
        if !error_str.is_empty() {
            error!("Executing command: ({purpose}) {:?}", line);
            error!("Command error: {:?}", error_str.trim());
        }
    }*/

    let mut commands = String::new();
    for line in command_buffer {
        for (idx, entry) in line.iter().enumerate() {
            commands.push_str(entry);
            if idx < line.len() - 1 {
                commands.push(' '); // Add space between entries
            }
        }
        let newline = "\n";
        commands.push_str(newline); // Add new-line at the end of the line
    }

    let Ok(mut child) = std::process::Command::new("/sbin/tc")
        .arg("-f")
        .arg("-batch")  // or "-force" if you want it to continue after errors
        .arg("-")       // read from stdin
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .inspect_err(|e| {
            error!("Failed to spawn tc command: {}", e);
        }) else {
            return;
        };

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        if let Err(e) = stdin.write_all(commands.as_bytes()) {
            error!("Failed to write to tc stdin: {}", e);
            return;
        }
    }

    let Ok(status) = child.wait() else {
        error!("Failed to wait for tc command to finish");
        return;
    };
    if !status.success() {
        eprintln!("tc command failed with status: {}", status);
    }
}

pub(crate) fn write_command_file(path: &Path, commands: &Vec<Vec<String>>) -> bool {
    let Ok(f) = File::create(path) else {
        error!("Failed to create output file: {}", path.display());
        return true;
    };
    let mut f = BufWriter::new(f);
    for line in commands {
        for (idx, entry) in line.iter().enumerate() {
            if let Err(e) = f.write_all(entry.as_bytes()) {
                error!("Failed to write to output file: {}", e);
                return true;
            }
            if idx < line.len() - 1 {
                if let Err(e) = f.write_all(b" ") {
                    error!("Failed to write space to output file: {}", e);
                    return true;
                }
            }
        }
        let newline = "\n";
        if let Err(e) = f.write_all(newline.as_bytes()) {
            error!("Failed to write newline to output file: {}", e);
            return true;
        }
    }
    false
}