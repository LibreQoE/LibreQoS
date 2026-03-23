use lqos_bus::TcHandle;
use parking_lot::Mutex;
use std::collections::HashSet;
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

#[derive(Clone, Debug)]
pub(crate) struct ExecuteResult {
    pub(crate) ok: bool,
    pub(crate) duration_ms: u64,
    pub(crate) failure_summary: Option<String>,
}

/// Lightweight host-memory snapshot used by Bakery safety checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemorySnapshot {
    /// Total installed RAM in bytes.
    pub total_bytes: u64,
    /// Currently available RAM in bytes.
    pub available_bytes: u64,
}

fn format_numbered_lines(lines: &str, starting_line_number: usize) -> String {
    let mut numbered = String::new();
    for (i, line) in lines.lines().enumerate() {
        numbered.push_str(&format!("{:>4}: {}\n", starting_line_number + i, line));
    }
    numbered
}

fn run_tc_batch(path: &Path, purpose: &str) -> Result<std::process::Output, String> {
    std::process::Command::new("/sbin/tc")
        .args(["-f", "-batch", path.to_str().unwrap_or_default()])
        .output()
        .map_err(|_| format!("Failed to execute tc batch command for {purpose}."))
}

pub(crate) fn read_memory_snapshot() -> Result<MemorySnapshot, String> {
    let raw = std::fs::read_to_string("/proc/meminfo")
        .map_err(|e| format!("Failed to read /proc/meminfo: {e}"))?;
    parse_memory_snapshot(&raw)
}

fn parse_memory_snapshot(raw: &str) -> Result<MemorySnapshot, String> {
    let mut total_kib = None;
    let mut available_kib = None;

    for line in raw.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let kib = value
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<u64>().ok());
        match key.trim() {
            "MemTotal" => total_kib = kib,
            "MemAvailable" => available_kib = kib,
            _ => {}
        }
    }

    let total_kib = total_kib.ok_or_else(|| "MemTotal missing from /proc/meminfo".to_string())?;
    let available_kib =
        available_kib.ok_or_else(|| "MemAvailable missing from /proc/meminfo".to_string())?;

    Ok(MemorySnapshot {
        total_bytes: total_kib.saturating_mul(1024),
        available_bytes: available_kib.saturating_mul(1024),
    })
}

fn memory_guard_failure_summary(
    snapshot: MemorySnapshot,
    min_available_bytes: u64,
    purpose: &str,
    chunk_number: usize,
    total_chunks: usize,
    phase: &str,
) -> String {
    format!(
        "Bakery memory guard stopped {purpose} {phase} chunk {}/{}: available memory {} bytes is below safety floor {} bytes (total RAM {} bytes).",
        chunk_number,
        total_chunks,
        snapshot.available_bytes,
        min_available_bytes,
        snapshot.total_bytes
    )
}

pub(crate) fn read_live_qdisc_handle_majors(interface: &str) -> Result<HashSet<u16>, String> {
    let output = std::process::Command::new("/sbin/tc")
        .args(["-s", "-j", "qdisc", "show", "dev", interface])
        .output()
        .map_err(|e| format!("Failed to snapshot live qdiscs on {interface}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to snapshot live qdiscs on {interface}: {}",
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Live qdisc snapshot on {interface} was not UTF-8: {e}"))?;

    parse_live_qdisc_handle_majors(&stdout)
        .map_err(|e| format!("Failed to parse live qdisc snapshot on {interface}: {e}"))
}

fn parse_live_qdisc_handle_majors(raw_json: &str) -> Result<HashSet<u16>, String> {
    let parsed = serde_json::from_str::<serde_json::Value>(raw_json)
        .map_err(|e| format!("invalid JSON: {e}"))?;
    let items = parsed
        .as_array()
        .ok_or_else(|| "expected JSON array from tc qdisc show -j".to_string())?;

    let mut handles = HashSet::new();
    for item in items {
        let Some(handle) = item.get("handle").and_then(|value| value.as_str()) else {
            continue;
        };
        let Ok(tc_handle) = TcHandle::from_string(handle) else {
            continue;
        };
        let (major, _) = tc_handle.get_major_minor();
        if major != 0 {
            handles.insert(major);
        }
    }

    Ok(handles)
}

pub(crate) fn execute_in_memory(command_buffer: &[Vec<String>], purpose: &str) -> ExecuteResult {
    execute_in_memory_chunked(
        command_buffer,
        purpose,
        command_buffer.len().max(1),
        None,
        |_, _, _, _| {},
    )
}

pub(crate) fn execute_in_memory_chunked<F>(
    command_buffer: &[Vec<String>],
    purpose: &str,
    chunk_size: usize,
    memory_guard_min_available_bytes: Option<u64>,
    mut on_progress: F,
) -> ExecuteResult
where
    F: FnMut(usize, usize, usize, usize),
{
    let started = std::time::Instant::now();
    let _lock = FILE_LOCK.lock();
    info!(
        "Bakery: Executing in-memory commands: {} lines, for {purpose}",
        command_buffer.len()
    );

    let full_path = Path::new("/tmp/lqos_bakery_commands.txt");
    let Some(_) = write_command_file(full_path, command_buffer) else {
        error!("Failed to write commands to file for {purpose}");
        return ExecuteResult {
            ok: false,
            duration_ms: started.elapsed().as_millis() as u64,
            failure_summary: Some(format!("Failed to write commands to file for {purpose}")),
        };
    };

    if command_buffer.is_empty() {
        return ExecuteResult {
            ok: true,
            duration_ms: started.elapsed().as_millis() as u64,
            failure_summary: None,
        };
    }

    let chunk_size = chunk_size.max(1);
    let total_chunks = command_buffer.len().div_ceil(chunk_size);
    let chunk_path = Path::new("/tmp/lqos_bakery_commands_chunk.txt");
    let mut completed_commands = 0usize;
    let mut completed_chunks = 0usize;

    for chunk in command_buffer.chunks(chunk_size) {
        if let Some(min_available_bytes) = memory_guard_min_available_bytes {
            match read_memory_snapshot() {
                Ok(snapshot) if snapshot.available_bytes < min_available_bytes => {
                    let summary = memory_guard_failure_summary(
                        snapshot,
                        min_available_bytes,
                        purpose,
                        completed_chunks + 1,
                        total_chunks,
                        "before",
                    );
                    error!("{summary}");
                    return ExecuteResult {
                        ok: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        failure_summary: Some(summary),
                    };
                }
                Ok(_) => {}
                Err(err) => {
                    let summary = format!(
                        "Bakery memory guard could not read host memory state before chunk {}/{} for {purpose}: {err}",
                        completed_chunks + 1,
                        total_chunks
                    );
                    error!("{summary}");
                    return ExecuteResult {
                        ok: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        failure_summary: Some(summary),
                    };
                }
            }
        }

        let global_line_start = completed_commands + 1;
        let Some(lines) = write_command_file(chunk_path, chunk) else {
            error!("Failed to write chunked commands to file for {purpose}");
            return ExecuteResult {
                ok: false,
                duration_ms: started.elapsed().as_millis() as u64,
                failure_summary: Some(format!("Failed to write commands to file for {purpose}")),
            };
        };

        let output = match run_tc_batch(chunk_path, purpose) {
            Ok(output) => output,
            Err(message) => {
                error!(message);
                return ExecuteResult {
                    ok: false,
                    duration_ms: started.elapsed().as_millis() as u64,
                    failure_summary: Some(message),
                };
            }
        };

        let output_str = String::from_utf8_lossy(&output.stdout)
            .replace("Error: Exclusivity flag on, cannot modify.\n", "");
        if !output_str.is_empty() {
            error!("Command output for ({purpose}): {:?}", output_str.trim());
        }

        let error_str = String::from_utf8_lossy(&output.stderr);
        if !error_str.is_empty() {
            let numbered = format_numbered_lines(&lines, global_line_start);
            let chunk_line_end = global_line_start + chunk.len().saturating_sub(1);
            let detailed = format!(
                "Command error for ({purpose}): {}\nFailed chunk {}/{} (global lines {}-{})\nFull batch: {}\nChunk batch: {}\nChunk commands with global line numbers:\n{}",
                error_str.trim(),
                completed_chunks + 1,
                total_chunks,
                global_line_start,
                chunk_line_end,
                full_path.display(),
                chunk_path.display(),
                numbered
            );
            error!(detailed);

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
            let path_last = Path::new("/tmp/lqos_bakery_last_error.txt");
            if let Ok(mut f) = File::create(path_last) {
                let _ = f.write_all(detailed.as_bytes());
                let _ = f.flush();
            }
            return ExecuteResult {
                ok: false,
                duration_ms: started.elapsed().as_millis() as u64,
                failure_summary: Some(error_str.trim().to_string()),
            };
        }

        completed_commands += chunk.len();
        completed_chunks += 1;

        if let Some(min_available_bytes) = memory_guard_min_available_bytes {
            match read_memory_snapshot() {
                Ok(snapshot) if snapshot.available_bytes < min_available_bytes => {
                    let summary = memory_guard_failure_summary(
                        snapshot,
                        min_available_bytes,
                        purpose,
                        completed_chunks,
                        total_chunks,
                        "after",
                    );
                    error!("{summary}");
                    return ExecuteResult {
                        ok: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        failure_summary: Some(summary),
                    };
                }
                Ok(_) => {}
                Err(err) => {
                    let summary = format!(
                        "Bakery memory guard could not read host memory state after chunk {}/{} for {purpose}: {err}",
                        completed_chunks, total_chunks
                    );
                    error!("{summary}");
                    return ExecuteResult {
                        ok: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        failure_summary: Some(summary),
                    };
                }
            }
        }

        on_progress(
            completed_commands,
            command_buffer.len(),
            completed_chunks,
            total_chunks,
        );
    }

    ExecuteResult {
        ok: true,
        duration_ms: started.elapsed().as_millis() as u64,
        failure_summary: None,
    }
}

pub(crate) fn write_command_file(path: &Path, commands: &[Vec<String>]) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_live_qdisc_handle_majors_collects_non_zero_handles() {
        let raw = r#"
[
  { "kind": "mq", "handle": "7fff:", "parent": "root" },
  { "kind": "cake", "handle": "90f1:", "parent": "2:1039" },
  { "kind": "fq_codel", "handle": "50c0:", "parent": "8:24dd" },
  { "kind": "ingress", "handle": "ffff:", "parent": "ffff:fff1" },
  { "kind": "fq_codel", "handle": "0:", "parent": "3:20" }
]
"#;
        let handles = parse_live_qdisc_handle_majors(raw).expect("handles parsed");
        assert!(handles.contains(&0x7fff));
        assert!(handles.contains(&0x90f1));
        assert!(handles.contains(&0x50c0));
        assert!(handles.contains(&0xffff));
        assert!(!handles.contains(&0));
    }

    #[test]
    fn parse_live_qdisc_handle_majors_rejects_non_arrays() {
        let err = parse_live_qdisc_handle_majors(r#"{"handle":"90f1:"}"#)
            .expect_err("non-array should fail");
        assert!(err.contains("expected JSON array"));
    }

    #[test]
    fn parse_memory_snapshot_extracts_total_and_available() {
        let raw = "\
MemTotal:       32768000 kB
MemFree:         1024000 kB
MemAvailable:    8192000 kB
";
        let snapshot = parse_memory_snapshot(raw).expect("meminfo parsed");
        assert_eq!(snapshot.total_bytes, 32_768_000_u64 * 1024);
        assert_eq!(snapshot.available_bytes, 8_192_000_u64 * 1024);
    }

    #[test]
    fn parse_memory_snapshot_requires_memavailable() {
        let raw = "\
MemTotal:       32768000 kB
MemFree:         1024000 kB
";
        let err = parse_memory_snapshot(raw).expect_err("memavailable should be required");
        assert!(err.contains("MemAvailable"));
    }
}
