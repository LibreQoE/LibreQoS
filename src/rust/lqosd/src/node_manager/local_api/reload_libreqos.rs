use axum::Extension;
use tokio::task::spawn_blocking;
use tracing::info;
use crate::node_manager::auth::LoginResult;

pub async fn reload_libreqos(
    Extension(login) : Extension<LoginResult>,
) -> String {
    info!("Reloading LibreQoS");
    if let LoginResult::Admin = login {
        let result =spawn_blocking(|| lqos_config::load_libreqos()).await.unwrap();
        println!("{:?}", result);
        result.unwrap_or_else(|_| "Unable to reload LibreQoS".to_string())
    } else {
        "You must be an admin to reload LibreQoS".to_string()
    }
}