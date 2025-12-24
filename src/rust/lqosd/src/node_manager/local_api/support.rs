use axum::Json;
use lqos_support_tool::{SanityChecks, run_sanity_checks};

pub async fn run_sanity_check() -> Json<SanityChecks> {
    let mut status = run_sanity_checks(false).expect("Failed to run sanity checks");
    status.results.sort_by(|a, b| a.success.cmp(&b.success));
    Json(status)
}
