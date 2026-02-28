use crate::node_manager::auth::LoginResult;
use tokio::task::spawn_blocking;
use tracing::info;

pub async fn reload_libreqos_with_login(login: LoginResult) -> String {
    info!("Reloading LibreQoS");
    if let LoginResult::Admin = login {
        let Ok(outcome) = spawn_blocking(crate::reload_lock::try_reload_libreqos_locked).await else {
            return "Failed to spawn blocking thread".to_string();
        };
        match outcome {
            crate::reload_lock::ReloadExecOutcome::Success(message) => message,
            crate::reload_lock::ReloadExecOutcome::Busy => {
                "Reload already in progress".to_string()
            }
            crate::reload_lock::ReloadExecOutcome::Failed(_) => {
                "Unable to reload LibreQoS".to_string()
            }
        }
    } else {
        "You must be an admin to reload LibreQoS".to_string()
    }
}
