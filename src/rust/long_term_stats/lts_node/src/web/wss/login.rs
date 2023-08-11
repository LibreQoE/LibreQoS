use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use tokio::sync::mpsc::Sender;
use tracing::instrument;
use wasm_pipe_types::WasmResponse;

#[derive(Debug, Serialize, Clone)]
pub struct LoginResult {
    pub msg: String,
    pub token: String,
    pub name: String,
    pub license_key: String,
}

#[instrument(skip(license, username, password, tx, cnn))]
pub async fn on_login(license: &str, username: &str, password: &str, tx: Sender<WasmResponse>, cnn: Pool<Postgres>) -> Option<LoginResult> {
    let login = pgdb::try_login(cnn, license, username, password).await;
    if let Ok(login) = login {
        let lr = WasmResponse::LoginOk {
            token: login.token.clone(),
            name: login.name.clone(),
            license_key: license.to_string(),
        };
        tx.send(lr).await.unwrap();
        return Some(LoginResult {
            msg: "Login Ok".to_string(),
            token: login.token.to_string(),
            name: login.name.to_string(),
            license_key: license.to_string(),
        });
    } else {
        let lr = WasmResponse::LoginFail;
        tx.send(lr).await.unwrap();
    }
None
}

#[instrument(skip(token_id, tx, cnn))]
pub async fn on_token_auth(token_id: &str, tx: Sender<WasmResponse>, cnn: Pool<Postgres>) -> Option<LoginResult> {
    let login = pgdb::token_to_credentials(cnn, token_id).await;
    if let Ok(login) = login {
        let lr = WasmResponse::AuthOk {
            token: login.token.clone(),
            name: login.name.clone(),
            license_key: login.license.clone(),
        };
        tx.send(lr).await.unwrap();
        return Some(LoginResult {
            msg: "Login Ok".to_string(),
            token: login.token.to_string(),
            name: login.name.to_string(),
            license_key: login.license.to_string(),
        });
    } else {
        tx.send(WasmResponse::AuthFail).await.unwrap();
    }
    None
}