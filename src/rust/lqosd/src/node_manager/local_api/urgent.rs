use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
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

#[derive(Serialize)]
pub struct UrgentStatus {
    pub has_urgent: bool,
    pub count: usize,
}

#[derive(Serialize)]
pub struct UrgentItem {
    pub id: u64,
    pub ts: u64,
    pub source: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Serialize)]
pub struct UrgentList {
    pub items: Vec<UrgentItem>,
}

pub async fn urgent_status() -> (StatusCode, Json<UrgentStatus>) {
    let items = urgent::list();
    (
        StatusCode::OK,
        Json(UrgentStatus { has_urgent: !items.is_empty(), count: items.len() }),
    )
}

pub async fn urgent_list() -> (StatusCode, Json<UrgentList>) {
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
    (StatusCode::OK, Json(UrgentList { items }))
}

pub async fn urgent_clear(Path(id): Path<u64>) -> (StatusCode, &'static str) {
    let ok = urgent::clear(id);
    if ok {
        (StatusCode::OK, "OK")
    } else {
        (StatusCode::NOT_FOUND, "NOT_FOUND")
    }
}

pub async fn urgent_clear_all() -> (StatusCode, &'static str) {
    urgent::clear_all();
    (StatusCode::OK, "OK")
}
