use lqos_bus::TcHandle;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Get the current Unix timestamp in seconds
pub(crate) fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

static FILE_LOCK: Mutex<()> = Mutex::new(());
static LIVE_TC_SNAPSHOT_CACHE: LazyLock<Mutex<LiveTcSnapshotCache>> =
    LazyLock::new(|| Mutex::new(LiveTcSnapshotCache::new()));
const LIVE_TC_SNAPSHOT_MAX_AGE_MS: u64 = 250;

#[derive(Clone)]
struct TimedClassSnapshot {
    captured_at: Instant,
    snapshot: HashMap<TcHandle, LiveTcClassEntry>,
}

#[derive(Clone)]
struct TimedQdiscSnapshot {
    captured_at: Instant,
    handles: HashSet<u16>,
}

struct LiveTcSnapshotCache {
    class_snapshots: HashMap<String, TimedClassSnapshot>,
    qdisc_snapshots: HashMap<String, TimedQdiscSnapshot>,
}

impl LiveTcSnapshotCache {
    fn new() -> Self {
        Self {
            class_snapshots: HashMap::new(),
            qdisc_snapshots: HashMap::new(),
        }
    }

    fn invalidate(&mut self) {
        self.class_snapshots.clear();
        self.qdisc_snapshots.clear();
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveTcClassEntry {
    pub(crate) class_id: TcHandle,
    pub(crate) parent: Option<TcHandle>,
    pub(crate) leaf_qdisc_major: Option<u16>,
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

fn summarize_tc_batch_failure(output: &std::process::Output) -> Option<String> {
    if output.status.success() {
        return None;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();

    let status_summary = match output.status.code() {
        Some(code) => format!("tc batch exited with status {code}"),
        None => "tc batch terminated by signal".to_string(),
    };

    if stderr.is_empty() {
        Some(status_summary)
    } else {
        Some(format!("{status_summary}: {stderr}"))
    }
}

fn tc_batch_command_is_delete_only(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    matches!(
        (parts.next(), parts.next()),
        (Some("qdisc"), Some("del")) | (Some("class"), Some("del"))
    )
}

fn tc_batch_failure_is_ignorable_delete_absence(
    output: &std::process::Output,
    lines: &str,
) -> bool {
    if output.status.success() {
        return false;
    }

    if lines.trim().is_empty() || !lines.lines().all(tc_batch_command_is_delete_only) {
        return false;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        return false;
    }

    stderr.lines().all(|line| {
        let trimmed = line.trim().to_ascii_lowercase();
        trimmed.is_empty()
            || trimmed.starts_with("command failed ")
            || trimmed.starts_with("error: specified class not found")
            || trimmed.starts_with("error: cannot find specified qdisc on specified device")
            || trimmed.starts_with("rtnetlink answers: no such file or directory")
    })
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

#[allow(dead_code)]
pub(crate) fn invalidate_live_tc_snapshots() {
    let mut cache = LIVE_TC_SNAPSHOT_CACHE.lock();
    cache.invalidate();
}

fn read_live_qdisc_handle_majors_raw(interface: &str) -> Result<HashSet<u16>, String> {
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

pub(crate) fn read_live_qdisc_handle_majors(interface: &str) -> Result<HashSet<u16>, String> {
    let _lock = FILE_LOCK.lock();
    let mut cache = LIVE_TC_SNAPSHOT_CACHE.lock();
    let now = Instant::now();
    if let Some(entry) = cache.qdisc_snapshots.get(interface)
        && now.duration_since(entry.captured_at)
            <= Duration::from_millis(LIVE_TC_SNAPSHOT_MAX_AGE_MS)
    {
        return Ok(entry.handles.clone());
    }

    let handles = read_live_qdisc_handle_majors_raw(interface)?;
    cache.qdisc_snapshots.insert(
        interface.to_string(),
        TimedQdiscSnapshot {
            captured_at: now,
            handles: handles.clone(),
        },
    );
    Ok(handles)
}

fn read_live_class_snapshot_raw(
    interface: &str,
) -> Result<HashMap<TcHandle, LiveTcClassEntry>, String> {
    let output = std::process::Command::new("/sbin/tc")
        .args(["class", "show", "dev", interface])
        .output()
        .map_err(|e| format!("Failed to snapshot live classes on {interface}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to snapshot live classes on {interface}: {}",
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Live class snapshot on {interface} was not UTF-8: {e}"))?;

    parse_live_class_snapshot(&stdout)
        .map_err(|e| format!("Failed to parse live class snapshot on {interface}: {e}"))
}

pub(crate) fn read_live_class_snapshot(
    interface: &str,
) -> Result<HashMap<TcHandle, LiveTcClassEntry>, String> {
    let _lock = FILE_LOCK.lock();
    let mut cache = LIVE_TC_SNAPSHOT_CACHE.lock();
    let now = Instant::now();
    if let Some(entry) = cache.class_snapshots.get(interface)
        && now.duration_since(entry.captured_at)
            <= Duration::from_millis(LIVE_TC_SNAPSHOT_MAX_AGE_MS)
    {
        return Ok(entry.snapshot.clone());
    }

    let snapshot = read_live_class_snapshot_raw(interface)?;
    cache.class_snapshots.insert(
        interface.to_string(),
        TimedClassSnapshot {
            captured_at: now,
            snapshot: snapshot.clone(),
        },
    );
    Ok(snapshot)
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

fn parse_live_class_snapshot(raw: &str) -> Result<HashMap<TcHandle, LiveTcClassEntry>, String> {
    let mut snapshot = HashMap::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with("class ") {
            continue;
        }

        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 3 {
            return Err(format!("Malformed tc class line: {trimmed}"));
        }

        let class_id = TcHandle::from_string(tokens[2]).map_err(|e| {
            format!(
                "Invalid tc class handle {:?} in line {:?}: {:?}",
                tokens[2], trimmed, e
            )
        })?;

        let mut parent = None;
        let mut leaf_qdisc_major = None;
        let mut idx = 3usize;
        while idx < tokens.len() {
            match tokens[idx] {
                "parent" if idx + 1 < tokens.len() => {
                    if let Ok(handle) = TcHandle::from_string(tokens[idx + 1]) {
                        parent = Some(handle);
                    }
                    idx += 2;
                }
                "leaf" if idx + 1 < tokens.len() => {
                    if let Ok(handle) = TcHandle::from_string(tokens[idx + 1]) {
                        let (major, _) = handle.get_major_minor();
                        if major != 0 {
                            leaf_qdisc_major = Some(major);
                        }
                    }
                    idx += 2;
                }
                _ => idx += 1,
            }
        }

        snapshot.insert(
            class_id,
            LiveTcClassEntry {
                class_id,
                parent,
                leaf_qdisc_major,
            },
        );
    }

    Ok(snapshot)
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() && !stderr.trim().is_empty() {
            warn!("Command stderr for ({purpose}): {:?}", stderr.trim());
        }

        if tc_batch_failure_is_ignorable_delete_absence(&output, &lines) {
            debug!(
                "Bakery tolerated delete-only tc batch absence during {purpose}; targets were already gone"
            );
        } else if let Some(failure_summary) = summarize_tc_batch_failure(&output) {
            let numbered = format_numbered_lines(&lines, global_line_start);
            let chunk_line_end = global_line_start + chunk.len().saturating_sub(1);
            let detailed = format!(
                "Command error for ({purpose}): {}\nFailed chunk {}/{} (global lines {}-{})\nFull batch: {}\nChunk batch: {}\nChunk commands with global line numbers:\n{}",
                failure_summary,
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
                failure_summary: Some(failure_summary),
            };
        }

        completed_commands += chunk.len();
        completed_chunks += 1;
        LIVE_TC_SNAPSHOT_CACHE.lock().invalidate();

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
    use std::os::unix::process::ExitStatusExt;

    fn mock_tc_output(status: i32, stdout: &str, stderr: &str) -> std::process::Output {
        std::process::Output {
            status: std::process::ExitStatus::from_raw(status << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

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
    fn parse_live_class_snapshot_extracts_parent_and_leaf() {
        let raw = "\
class htb 1:da parent 1:4 leaf ddad: prio 3 rate 20Mbit ceil 100Mbit burst 1600b cburst 1600b
class htb 1:4 root rate 949Mbit ceil 950Mbit burst 1423b cburst 1425b
";
        let snapshot = parse_live_class_snapshot(raw).expect("snapshot parsed");
        let class_da = snapshot
            .get(&TcHandle::from_string("1:da").expect("valid class"))
            .expect("class 1:da present");
        assert_eq!(
            class_da.parent,
            Some(TcHandle::from_string("1:4").expect("valid parent"))
        );
        assert_eq!(class_da.leaf_qdisc_major, Some(0xddad));

        let class_root = snapshot
            .get(&TcHandle::from_string("1:4").expect("valid class"))
            .expect("class 1:4 present");
        assert_eq!(class_root.parent, None);
        assert_eq!(class_root.leaf_qdisc_major, None);
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

    #[test]
    fn summarize_tc_batch_failure_rejects_nonzero_exit_without_stderr() {
        let output = mock_tc_output(1, "", "");
        let summary = summarize_tc_batch_failure(&output).expect("nonzero exit should fail");
        assert!(summary.contains("status 1"));
    }

    #[test]
    fn summarize_tc_batch_failure_includes_stderr_with_exit_status() {
        let output = mock_tc_output(2, "", "RTNETLINK answers: Invalid argument\n");
        let summary = summarize_tc_batch_failure(&output).expect("stderr failure should be kept");
        assert!(summary.contains("status 2"));
        assert!(summary.contains("Invalid argument"));
    }

    #[test]
    fn summarize_tc_batch_failure_accepts_clean_success() {
        let output = mock_tc_output(0, "", "");
        assert!(summarize_tc_batch_failure(&output).is_none());
    }

    #[test]
    fn summarize_tc_batch_failure_accepts_success_with_stderr_warning() {
        let output = mock_tc_output(0, "", "Warning: sch_htb: quantum of class 10134 is big.\n");
        assert!(summarize_tc_batch_failure(&output).is_none());
    }

    #[test]
    fn ignorable_delete_absence_accepts_missing_delete_targets() {
        let output = mock_tc_output(
            1,
            "",
            "Error: Specified class not found.\nCommand failed /tmp/x:1\nRTNETLINK answers: No such file or directory\nCommand failed /tmp/x:2\n",
        );
        let lines = "qdisc del dev if0 parent 0x1:0x2000\nclass del dev if0 parent 0x1:0x3 classid 0x1:0x2000\n";
        assert!(tc_batch_failure_is_ignorable_delete_absence(&output, lines));
    }

    #[test]
    fn ignorable_delete_absence_rejects_non_delete_failures() {
        let output = mock_tc_output(1, "", "Error: HTB class in use.\n");
        let lines = "class del dev if0 parent 0x1:0x35 classid 0x1:0x39\n";
        assert!(!tc_batch_failure_is_ignorable_delete_absence(
            &output, lines
        ));
    }
}
