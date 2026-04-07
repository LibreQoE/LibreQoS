use serde::Serialize;

use lqos_bus::SchedulerProgressReport;

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
}

#[derive(Serialize, Debug, Clone)]
pub struct SchedulerDetails {
    pub available: bool,
    pub error: Option<String>,
    pub output: Option<String>,
    pub progress: Option<SchedulerProgressReport>,
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
    let available = is_scheduler_available();
    let error = scheduler_error();
    let progress = scheduler_progress_state();
    SchedulerStatus {
        available,
        error,
        progress,
    }
}

pub fn scheduler_details_data() -> SchedulerDetails {
    let status = scheduler_status_data();
    let output = scheduler_output();
    let mut body = String::new();
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
        details: body,
    }
}
