use axum::extract::ws::WebSocket;
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use wasm_pipe_types::WasmResponse;

use super::send_response;

#[derive(Debug, Serialize)]
pub struct LoginResult {
    pub msg: String,
    pub token: String,
    pub name: String,
    pub license_key: String,
}

pub async fn on_login(license: &str, username: &str, password: &str, socket: &mut WebSocket, cnn: Pool<Postgres>) -> Option<LoginResult> {
    let login = pgdb::try_login(cnn, license, username, password).await;
    if let Ok(login) = login {
        let lr = WasmResponse::LoginOk {
            token: login.token.clone(),
            name: login.name.clone(),
            license_key: license.to_string(),
        };
        send_response(socket, lr).await;
        return Some(LoginResult {
            msg: "Login Ok".to_string(),
            token: login.token.to_string(),
            name: login.name.to_string(),
            license_key: license.to_string(),
        });
    } else {
        let lr = WasmResponse::LoginFail;
        send_response(socket, lr).await;
    }
None
}

pub async fn on_token_auth(token_id: &str, socket: &mut WebSocket, cnn: Pool<Postgres>) -> Option<LoginResult> {
    let login = pgdb::token_to_credentials(cnn, token_id).await;
    if let Ok(login) = login {
        let lr = WasmResponse::AuthOk {
            token: login.token.clone(),
            name: login.name.clone(),
            license_key: login.license.clone(),
        };
        send_response(socket, lr).await;
        return Some(LoginResult {
            msg: "Login Ok".to_string(),
            token: login.token.to_string(),
            name: login.name.to_string(),
            license_key: login.license.to_string(),
        });
    } else {
        send_response(socket, WasmResponse::AuthFail).await;
    }
    None
}