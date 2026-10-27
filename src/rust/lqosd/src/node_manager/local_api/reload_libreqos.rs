use crate::node_manager::auth::LoginResult;
use tokio::task::spawn_blocking;
use tracing::info;

/// Requests an on-demand source refresh from the scheduler (the single apply
/// owner). The DPDK bridge is never restarted by this control.
pub async fn reload_libreqos_with_login(login: LoginResult) -> String {
    if let LoginResult::Admin = login {
        info!("Requesting LibreQoS reload");
        let Ok(result) = spawn_blocking(|| lqos_config::request_libreqos_reload("admin")).await
        else {
            return "Failed to spawn blocking thread".to_string();
        };
        result.unwrap_or_else(|_| "Unable to request the LibreQoS reload".to_string())
    } else {
        "You must be an admin to reload LibreQoS".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_only_login_is_refused() {
        let message = reload_libreqos_with_login(LoginResult::ReadOnly).await;
        assert_eq!(message, "You must be an admin to reload LibreQoS");
    }

    #[tokio::test]
    async fn denied_login_is_refused() {
        let message = reload_libreqos_with_login(LoginResult::Denied).await;
        assert_eq!(message, "You must be an admin to reload LibreQoS");
    }
}
