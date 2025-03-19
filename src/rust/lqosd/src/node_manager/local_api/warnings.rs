use crate::node_manager::WarningLevel;
use axum::Json;
use tracing::{error, info};

pub async fn get_global_warnings() -> Json<Vec<(WarningLevel, String)>> {
    let mut warnings = crate::node_manager::warnings::get_global_warnings();

    info!("Sanity checking configuration...");

    if let Ok(support_sanity) = lqos_support_tool::run_sanity_checks(false) {
        let found_warnings = support_sanity.results.iter().any(|c| !c.success);
        if found_warnings {
            warnings.push((WarningLevel::Error, "Support tool sanity checks failed. Please run the <a href='help.html'>support tool</a>.".to_string()));
        }
    } else {
        error!("Failed to run support tool sanity checks");
    }

    Json(warnings)
}
