use serde::Serialize;

use lqos_bus::SchedulerProgressReport;

use crate::node_manager::runtime_onboarding::runtime_onboarding_state;
use crate::tool_status::{
    is_scheduler_available, scheduler_error_message, scheduler_output_message,
    scheduler_progress_state,
};

// Remove ANSI escape sequences (basic CSI/OSC handling) for browser display
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B {
            // ESC sequence
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] as char {
                '[' => {
                    // CSI: ESC [ ... final byte 0x40..=0x7E
                    i += 1;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if (0x40..=0x7E).contains(&b) {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                ']' => {
                    // OSC: ESC ] ... BEL (0x07) or ESC \
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1B {
                            // ESC
                            if i + 1 < bytes.len() && bytes[i + 1] as char == '\\' {
                                i += 2; // ESC \
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                _ => {
                    // Other ESC-seq: skip next char at least
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[derive(Serialize, Debug, Clone)]
pub struct SchedulerStatus {
    pub available: bool,
    pub error: Option<String>,
    pub progress: Option<SchedulerProgressReport>,
    pub setup_required: bool,
    pub setup_message: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct SchedulerDetails {
    pub available: bool,
    pub error: Option<String>,
    pub output: Option<String>,
    pub progress: Option<SchedulerProgressReport>,
    pub setup_required: bool,
    pub setup_message: Option<String>,
    pub details: String,
}

fn scheduler_error() -> Option<String> {
    scheduler_error_message().and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(strip_ansi(&t))
        }
    })
}

fn scheduler_output() -> Option<String> {
    scheduler_output_message().and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(strip_ansi(&t))
        }
    })
}

pub fn scheduler_status_data() -> SchedulerStatus {
    let onboarding = runtime_onboarding_state();
    if onboarding.required {
        return SchedulerStatus {
            available: false,
            error: None,
            progress: None,
            setup_required: true,
            setup_message: Some(onboarding.summary),
        };
    }

    let available = is_scheduler_available();
    let error = scheduler_error();
    let progress = scheduler_progress_state();
    SchedulerStatus {
        available,
        error,
        progress,
        setup_required: false,
        setup_message: None,
    }
}

pub fn scheduler_details_data() -> SchedulerDetails {
    let status = scheduler_status_data();
    let output = scheduler_output();
    let mut body = String::new();
    if status.setup_required {
        let message = status.setup_message.clone().unwrap_or_else(|| {
            "Choose a topology source in Complete Setup before expecting scheduler activity."
                .to_string()
        });
        body.push_str("Scheduler status: setup required\n\n");
        body.push_str("LibreQoS runtime onboarding is incomplete.\n");
        body.push_str(&message);
        body.push('\n');
        return SchedulerDetails {
            available: false,
            error: None,
            output: None,
            progress: None,
            setup_required: true,
            setup_message: Some(message),
            details: body,
        };
    }

    body.push_str(&format!("Scheduler available: {}\n\n", status.available));
    match status.progress.as_ref() {
        Some(progress) => {
            body.push_str("Current progress:\n");
            body.push_str(&format!(
                "- Active: {}\n- Phase: {}\n- Step: {}/{}\n- Percent: {}%\n",
                progress.active,
                progress.phase_label,
                progress.step_index,
                progress.step_count,
                progress.percent
            ));
            if let Some(updated_unix) = progress.updated_unix {
                body.push_str(&format!("- Updated Unix: {}\n", updated_unix));
            }
            body.push('\n');
        }
        None => {
            body.push_str("No scheduler progress reported.\n\n");
        }
    }
    match status.error.as_ref() {
        Some(err) => {
            body.push_str("Reported error:\n");
            body.push_str(err);
            body.push('\n');
        }
        None => {
            body.push_str("No scheduler error reported.\n");
        }
    }
    body.push('\n');
    match output.as_ref() {
        Some(text) => {
            body.push_str("Recent output:\n");
            body.push_str(text);
            body.push('\n');
        }
        None => {
            body.push_str("No recent scheduler output recorded.\n");
        }
    }
    body.push_str("\n(Additional scheduler diagnostics not available.)\n");

    SchedulerDetails {
        available: status.available,
        error: status.error,
        output,
        progress: status.progress,
        setup_required: false,
        setup_message: None,
        details: body,
    }
}

#[cfg(test)]
mod tests {
    use super::{scheduler_details_data, scheduler_status_data};
    use crate::test_support::runtime_config_test_lock;
    use crate::tool_status::{
        scheduler_error, scheduler_output, scheduler_progress, scheduler_seen,
    };
    use lqos_bus::SchedulerProgressReport;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "libreqos-scheduler-status-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn write_scheduler_test_config(runtime_dir: &Path) -> PathBuf {
        let config_path = runtime_dir.join("lqos.conf");
        let runtime_dir_display = runtime_dir.to_string_lossy();
        let state_dir_display = runtime_dir.join("state").to_string_lossy().into_owned();
        let raw = include_str!("../../../../lqos_config/src/etc/v15/example.toml")
            .replacen(
                "lqos_directory = \"/opt/libreqos/src\"",
                &format!("lqos_directory = {:?}", runtime_dir_display),
                1,
            )
            .replacen(
                "state_directory = \"/opt/libreqos/state\"",
                &format!("state_directory = {:?}", state_dir_display),
                1,
            )
            .replacen("enable_splynx = false", "enable_splynx = true", 1);
        fs::write(&config_path, raw).expect("write scheduler test config");
        config_path
    }

    struct SchedulerStatusTestContext {
        _guard: std::sync::MutexGuard<'static, ()>,
        old_lqos_config: Option<OsString>,
        old_lqos_directory: Option<OsString>,
        runtime_dir: PathBuf,
    }

    impl SchedulerStatusTestContext {
        fn new(label: &str) -> Self {
            let guard = runtime_config_test_lock()
                .lock()
                .expect("scheduler status test lock should not be poisoned");
            let runtime_dir = unique_test_dir(label);
            fs::create_dir_all(runtime_dir.join("state")).expect("create runtime state directory");
            let config_path = write_scheduler_test_config(&runtime_dir);
            let old_lqos_config = std::env::var_os("LQOS_CONFIG");
            let old_lqos_directory = std::env::var_os("LQOS_DIRECTORY");
            unsafe {
                std::env::set_var("LQOS_CONFIG", &config_path);
                std::env::set_var("LQOS_DIRECTORY", &runtime_dir);
            }
            lqos_config::clear_cached_config();
            Self {
                _guard: guard,
                old_lqos_config,
                old_lqos_directory,
                runtime_dir,
            }
        }
    }

    impl Drop for SchedulerStatusTestContext {
        fn drop(&mut self) {
            scheduler_error(None);
            scheduler_output(None);
            scheduler_progress(None);
            match &self.old_lqos_config {
                Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
                None => unsafe { std::env::remove_var("LQOS_CONFIG") },
            }
            match &self.old_lqos_directory {
                Some(value) => unsafe { std::env::set_var("LQOS_DIRECTORY", value) },
                None => unsafe { std::env::remove_var("LQOS_DIRECTORY") },
            }
            lqos_config::clear_cached_config();
            let _ = fs::remove_dir_all(&self.runtime_dir);
        }
    }

    #[test]
    fn scheduler_status_surfaces_validation_failure_for_ui_contract() {
        let _context = SchedulerStatusTestContext::new("validation-failure");
        scheduler_seen();
        scheduler_error(Some(
            "Scheduled shaping refresh blocked by validation: duplicate IPv4".to_string(),
        ));
        scheduler_output(Some(
            "Scheduled shaping refresh blocked by validation: duplicate IPv4".to_string(),
        ));
        scheduler_progress(Some(SchedulerProgressReport {
            active: false,
            phase: "validation_failed".to_string(),
            phase_label: "Scheduler validation failed".to_string(),
            step_index: 5,
            step_count: 5,
            percent: 100,
            updated_unix: Some(1_234_567_890),
        }));

        let status = scheduler_status_data();
        let details = scheduler_details_data();

        assert!(status.available);
        assert_eq!(
            status.error.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert_eq!(
            status
                .progress
                .as_ref()
                .map(|progress| progress.phase.as_str()),
            Some("validation_failed")
        );
        assert!(!status.setup_required);

        assert!(details.available);
        assert_eq!(
            details.error.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert_eq!(
            details.output.as_deref(),
            Some("Scheduled shaping refresh blocked by validation: duplicate IPv4")
        );
        assert!(details.details.contains("Reported error:"));
        assert!(details.details.contains("Scheduler validation failed"));
    }
}
