use crate::node_manager::auth::LoginResult;
use tokio::task::spawn_blocking;
use tracing::info;

pub async fn reload_libreqos_with_login(login: LoginResult) -> String {
    info!("Reloading LibreQoS");
    if let LoginResult::Admin = login {
        let Ok(result) = spawn_blocking(lqos_config::load_libreqos).await else {
            return "Failed to spawn blocking thread".to_string();
        };
        //println!("{:?}", result);
        result.unwrap_or_else(|_| "Unable to reload LibreQoS".to_string())
    } else {
        "You must be an admin to reload LibreQoS".to_string()
    }
}
