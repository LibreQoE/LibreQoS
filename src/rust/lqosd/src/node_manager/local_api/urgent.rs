use serde::Serialize;

use crate::urgent;

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

#[derive(Debug, Serialize)]
pub struct UrgentStatus {
    pub has_urgent: bool,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct UrgentItem {
    pub id: u64,
    pub ts: u64,
    pub source: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UrgentList {
    pub items: Vec<UrgentItem>,
}

pub fn urgent_status_data() -> UrgentStatus {
    let items = urgent::list();
    UrgentStatus {
        has_urgent: !items.is_empty(),
        count: items.len(),
    }
}

pub fn urgent_list_data() -> UrgentList {
    let items = urgent::list();
    let items: Vec<UrgentItem> = items
        .into_iter()
        .map(|i| UrgentItem {
            id: i.id,
            ts: i.ts,
            source: format!("{:?}", i.source),
            severity: format!("{:?}", i.severity),
            code: i.code,
            message: strip_ansi(&i.message),
            context: i.context,
        })
        .collect();
    UrgentList { items }
}

pub fn urgent_clear_id(id: u64) -> bool {
    urgent::clear(id)
}

pub fn urgent_clear_all_data() {
    urgent::clear_all();
}
