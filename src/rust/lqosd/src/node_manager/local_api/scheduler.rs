use serde::Serialize;

use crate::tool_status::{is_scheduler_available, scheduler_error_message};

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
}

#[derive(Serialize, Debug, Clone)]
pub struct SchedulerDetails {
    pub available: bool,
    pub error: Option<String>,
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

pub fn scheduler_status_data() -> SchedulerStatus {
    let available = is_scheduler_available();
    let error = scheduler_error();
    SchedulerStatus { available, error }
}

pub fn scheduler_details_data() -> SchedulerDetails {
    let status = scheduler_status_data();
    let mut body = String::new();
    body.push_str(&format!("Scheduler available: {}\n\n", status.available));
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
    body.push_str("\n(Additional scheduler diagnostics not available.)\n");

    SchedulerDetails {
        available: status.available,
        error: status.error,
        details: body,
    }
}
