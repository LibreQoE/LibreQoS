use axum::extract::ws::{WebSocket, Message};
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct LoginResult {
    pub msg: String,
    pub token: String,
    pub name: String,
    pub license_key: String,
}

pub async fn on_login(json: &Value, socket: &mut WebSocket, cnn: Pool<Postgres>) -> Option<LoginResult> {
    if let (
        Some(Value::String(license)),
        Some(Value::String(username)),
        Some(Value::String(password)),
    ) = (
        json.get("license"),
        json.get("username"),
        json.get("password"),
    ) {
        let login = pgdb::try_login(cnn, license, username, password).await;
        if let Ok(login) = login {
            let lr = LoginResult {
                msg: "loginOk".to_string(),
                token: login.token,
                name: login.name,
                license_key: license.to_string(),
            };
            if let Ok(login) = serde_json::to_string(&lr) {
                let msg = Message::Text(login);
                socket.send(msg).await.unwrap();
                return Some(lr);
            }
        } else {
            let lr = LoginResult {
                msg: "loginFail".to_string(),
                token: String::new(),
                name: String::new(),
                license_key: license.to_string(),
            };
            if let Ok(login) = serde_json::to_string(&lr) {
                let msg = Message::Text(login);
                socket.send(msg).await.unwrap();
            }
        }
    }
    None
}

pub async fn on_token_auth(json: &Value, socket: &mut WebSocket, cnn: Pool<Postgres>) -> Option<LoginResult> {
    let token_id = json.get("token").unwrap().as_str().unwrap();
    let login = pgdb::token_to_credentials(cnn, token_id).await;
    if let Ok(login) = login {
        let lr = LoginResult {
            msg: "authOk".to_string(),
            token: login.token,
            name: login.name,
            license_key: login.license,
        };
        if let Ok(login) = serde_json::to_string(&lr) {
            let msg = Message::Text(login);
            socket.send(msg).await.unwrap();
            return Some(lr);
        }
    } else {
        let lr = LoginResult {
            msg: "authFail".to_string(),
            token: String::new(),
            name: String::new(),
            license_key: String::new(),
        };
        if let Ok(login) = serde_json::to_string(&lr) {
            let msg = Message::Text(login);
            socket.send(msg).await.unwrap();
        }
    }
    None
}