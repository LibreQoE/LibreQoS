use crate::config::StormguardConfig;
use allocative::Allocative;
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use lqos_bus::{StormguardDebugDirection, StormguardDebugEntry};
use parking_lot::Mutex;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, warn};

const SCHEMA_VERSION: &str = "1";
const LOG_CHANNEL_CAPACITY: usize = 8;
const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
const HEADER: [&str; 35] = [
    "schema_version",
    "timestamp_unix_ms",
    "site",
    "direction",
    "mode",
    "strategy",
    "queue_mbps",
    "min_mbps",
    "max_mbps",
    "throughput_mbps",
    "throughput_ma_mbps",
    "retransmit_fraction",
    "retransmit_ma",
    "passive_rtt_ms",
    "active_ping_rtt_ms",
    "active_ping_target",
    "active_ping_weight",
    "effective_rtt_ms",
    "rtt_ma_ms",
    "baseline_rtt_ms",
    "delay_ms",
    "passive_rtt_flow_count",
    "decision_score",
    "candidate_action",
    "candidate_target_mbps",
    "decision_reason",
    "decision_blocker",
    "state",
    "cooldown_remaining_secs",
    "last_attempt_action",
    "last_attempt_target_mbps",
    "last_attempt_outcome",
    "last_attempt_unix_ms",
    "last_attempt_error",
    "rtt_source",
];

#[derive(Allocative)]
pub enum LogCommand {
    Snapshot {
        entries: Vec<StormguardDebugEntry>,
        mode: String,
        timestamp_unix_ms: u64,
    },
}

#[derive(Clone)]
pub struct DatalogHandle {
    sender: Sender<LogCommand>,
    health: Arc<Mutex<Option<String>>>,
    path: PathBuf,
}

impl DatalogHandle {
    pub fn try_send(&self, command: LogCommand) -> Result<(), TrySendError<LogCommand>> {
        self.sender.try_send(command)
    }

    pub fn last_error(&self) -> Option<String> {
        self.health.lock().clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn start_datalog(config: &StormguardConfig) -> anyhow::Result<DatalogHandle> {
    let log_path = config
        .log_filename
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("StormGuard diagnostic log path is not configured"))?;
    prepare_log(Path::new(log_path), MAX_LOG_BYTES)?;
    let (tx, rx) = bounded(LOG_CHANNEL_CAPACITY);
    let health = Arc::new(Mutex::new(None));
    let thread_health = Arc::clone(&health);
    let path = PathBuf::from(log_path);
    let thread_path = path.clone();
    std::thread::Builder::new()
        .name("StormguardLogger".to_string())
        .spawn(move || run_datalog(rx, thread_path, MAX_LOG_BYTES, thread_health))?;
    Ok(DatalogHandle {
        sender: tx,
        health,
        path,
    })
}

fn prepare_log(path: &Path, max_bytes: u64) -> anyhow::Result<()> {
    rotate_if_needed(path, max_bytes)?;
    let needs_header = path.metadata().map_or(true, |metadata| metadata.len() == 0);
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    if needs_header {
        let mut writer = csv::WriterBuilder::new()
            .delimiter(b';')
            .has_headers(false)
            .from_writer(file);
        writer.write_record(HEADER)?;
        writer.flush()?;
    }
    Ok(())
}

fn run_datalog(
    rx: Receiver<LogCommand>,
    path: PathBuf,
    max_bytes: u64,
    health: Arc<Mutex<Option<String>>>,
) {
    while let Ok(command) = rx.recv() {
        match write_command(&path, max_bytes, command) {
            Ok(()) => *health.lock() = None,
            Err(error) => {
                let error = format!("Failed to write StormGuard diagnostic log: {error}");
                *health.lock() = Some(error.clone());
                error!("{error}");
            }
        }
    }
}

fn write_command(path: &Path, max_bytes: u64, command: LogCommand) -> anyhow::Result<()> {
    rotate_if_needed(path, max_bytes)?;
    let needs_header = path.metadata().map_or(true, |metadata| metadata.len() == 0);
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut writer = csv::WriterBuilder::new()
        .delimiter(b';')
        .has_headers(false)
        .from_writer(file);
    if needs_header {
        writer.write_record(HEADER)?;
    }

    match command {
        LogCommand::Snapshot {
            entries,
            mode,
            timestamp_unix_ms,
        } => {
            for entry in entries {
                write_direction(
                    &mut writer,
                    timestamp_unix_ms,
                    &mode,
                    &entry,
                    "download",
                    &entry.download,
                    entry.passive_rtt_flow_counts.0,
                )?;
                write_direction(
                    &mut writer,
                    timestamp_unix_ms,
                    &mode,
                    &entry,
                    "upload",
                    &entry.upload,
                    entry.passive_rtt_flow_counts.1,
                )?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn write_direction(
    writer: &mut csv::Writer<File>,
    timestamp_unix_ms: u64,
    mode: &str,
    entry: &StormguardDebugEntry,
    direction_name: &str,
    direction: &StormguardDebugDirection,
    rtt_flow_count: u32,
) -> csv::Result<()> {
    let fields = [
        SCHEMA_VERSION.to_string(),
        timestamp_unix_ms.to_string(),
        entry.site.clone(),
        direction_name.to_string(),
        mode.to_string(),
        direction.strategy.clone(),
        direction.queue_mbps.to_string(),
        direction.min_mbps.to_string(),
        direction.max_mbps.to_string(),
        direction.throughput_mbps.to_string(),
        optional(direction.throughput_ma_mbps),
        optional(direction.retrans),
        optional(direction.retrans_ma),
        optional(direction.passive_rtt_ms),
        optional(direction.active_ping_rtt_ms),
        entry.active_ping_target.clone(),
        entry.active_ping_weight.to_string(),
        optional(direction.rtt),
        optional(direction.rtt_ma),
        optional(direction.baseline_rtt_ms),
        optional(direction.delay_ms),
        rtt_flow_count.to_string(),
        optional(direction.decision_score),
        direction.candidate_action.clone().unwrap_or_default(),
        optional(direction.candidate_target_mbps),
        direction.decision_reason.clone(),
        direction.decision_blocker.clone().unwrap_or_default(),
        direction.state.clone(),
        optional(direction.cooldown_remaining_secs),
        direction.last_attempt_action.clone().unwrap_or_default(),
        optional(direction.last_attempt_target_mbps),
        direction.last_attempt_outcome.clone().unwrap_or_default(),
        optional(direction.last_attempt_unix_ms),
        direction.last_attempt_error.clone().unwrap_or_default(),
        direction.rtt_source.clone(),
    ];
    writer.write_record(fields)
}

fn optional<T: ToString>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn rotate_if_needed(path: &Path, max_bytes: u64) -> anyhow::Result<()> {
    let Ok(metadata) = path.metadata() else {
        return Ok(());
    };
    if metadata.len() < max_bytes {
        return Ok(());
    }

    let backup = backup_path(path);
    if backup.exists() {
        std::fs::remove_file(&backup)?;
    }
    std::fs::rename(path, &backup)?;
    warn!("Rotated StormGuard diagnostic log to {}", backup.display());
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let mut backup: OsString = path.as_os_str().to_owned();
    backup.push(".1");
    PathBuf::from(backup)
}

#[cfg(test)]
mod tests {
    use super::{HEADER, LogCommand, backup_path, write_command};
    use lqos_bus::{StormguardDebugDirection, StormguardDebugEntry};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "stormguard-{name}-{}-{nonce}.csv",
            std::process::id()
        ))
    }

    fn direction() -> StormguardDebugDirection {
        StormguardDebugDirection {
            queue_mbps: 50,
            min_mbps: 10,
            max_mbps: 100,
            throughput_mbps: 42.5,
            throughput_ma_mbps: None,
            retrans: None,
            retrans_ma: None,
            rtt: Some(31.0),
            rtt_ma: None,
            passive_rtt_ms: Some(31.0),
            active_ping_rtt_ms: None,
            baseline_rtt_ms: None,
            delay_ms: None,
            strategy: "delay_probe".to_string(),
            last_action: None,
            last_action_age_secs: None,
            state: "Running".to_string(),
            cooldown_remaining_secs: None,
            saturation_current: "High".to_string(),
            saturation_max: "Medium".to_string(),
            can_increase: true,
            can_decrease: true,
            decision_score: None,
            candidate_action: None,
            candidate_target_mbps: None,
            decision_reason: "No action; observing".to_string(),
            decision_blocker: None,
            last_attempt_action: None,
            last_attempt_target_mbps: None,
            last_attempt_outcome: None,
            last_attempt_unix_ms: None,
            last_attempt_error: None,
            rtt_source: "passive".to_string(),
        }
    }

    fn command(timestamp_unix_ms: u64) -> LogCommand {
        LogCommand::Snapshot {
            entries: vec![StormguardDebugEntry {
                site: "Test;Site".to_string(),
                download: direction(),
                upload: direction(),
                active_ping_target: String::new(),
                active_ping_weight: 0.0,
                passive_rtt_flow_counts: (3, 2),
            }],
            mode: "live".to_string(),
            timestamp_unix_ms,
        }
    }

    #[test]
    fn writes_semicolon_rows_with_empty_optional_fields_and_appends() {
        let path = test_path("append");
        write_command(&path, u64::MAX, command(1)).unwrap();
        write_command(&path, u64::MAX, command(2)).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with("schema_version;timestamp_unix_ms;site;direction;"));
        assert_eq!(contents.lines().count(), 5);
        assert!(contents.contains("\"Test;Site\""));
        assert!(!contents.contains("Some("));
        assert_eq!(contents.matches("schema_version;").count(), 1);

        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .from_path(&path)
            .unwrap();
        assert_eq!(reader.headers().unwrap().iter().collect::<Vec<_>>(), HEADER);
        let rows = reader.records().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row.len() == HEADER.len()));
        let download = &rows[0];
        assert_eq!(&download[0], "1");
        assert_eq!(&download[1], "1");
        assert_eq!(&download[2], "Test;Site");
        assert_eq!(&download[3], "download");
        assert_eq!(&download[4], "live");
        assert_eq!(&download[10], "");
        assert_eq!(&download[21], "3");
        assert_eq!(&download[22], "");
        assert_eq!(&download[34], "passive");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rotates_to_one_backup_and_starts_a_new_file() {
        let path = test_path("rotation");
        write_command(&path, u64::MAX, command(1)).unwrap();
        write_command(&path, 1, command(2)).unwrap();

        assert!(backup_path(&path).exists());
        let current = std::fs::read_to_string(&path).unwrap();
        assert_eq!(current.lines().count(), 3);

        write_command(&path, 1, command(3)).unwrap();
        let backup = std::fs::read_to_string(backup_path(&path)).unwrap();
        assert!(backup.lines().skip(1).all(|line| line.starts_with("1;2;")));
        assert!(!backup.contains(";1;Test"));
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.lines().skip(1).all(|line| line.starts_with("1;3;")));
        let _ = std::fs::remove_file(backup_path(&path));
        let _ = std::fs::remove_file(path);
    }
}
