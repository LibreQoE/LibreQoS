//! Provides authentication for the Node Manager.
//! This is designed to be broadly compatible with the original
//! cookie-based system but now uses an Axum layer to be largely
//! invisible.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::Cookie;
use lqos_config::{UserRole, WebUsers, load_config};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::warn;

const COOKIE_PATH: &str = "User-Token";

static WEB_USERS: Lazy<Mutex<Option<WebUsers>>> = Lazy::new(|| Mutex::new(None));
pub static FIRST_LOAD: AtomicU64 = AtomicU64::new(0);

fn record_first_login_timestamp_if_needed() {
    let config = match load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            warn!("Unable to load config to record first-login timestamp: {e}");
            return;
        }
    };

    let path = Path::new(&config.lqos_directory).join(".fl");
    if path.exists() {
        if FIRST_LOAD.load(Relaxed) != 0 {
            return;
        }
        let Ok(str) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(ts_int) = str.trim().parse::<u64>() else {
            return;
        };
        FIRST_LOAD.store(ts_int, Relaxed);
        return;
    }

    let ts = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => {
            warn!("SystemTime before UNIX_EPOCH when recording first-login timestamp: {e:?}");
            return;
        }
    };

    if let Err(e) = std::fs::write(&path, ts.to_string()) {
        warn!("Failed to write first-login timestamp to {:?}: {e}", path);
    }
}

pub async fn get_username(jar: &CookieJar) -> String {
    let lock = WEB_USERS.lock().await;
    if let Some(users) = &*lock {
        if let Some(token) = jar.get(COOKIE_PATH) {
            return users.get_username(token.value());
        }
    }

    return "Anonymous".to_string();
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum LoginResult {
    Admin,
    ReadOnly,
    Denied,
}

async fn check_login(jar: &CookieJar, users: &WebUsers) -> LoginResult {
    if let Some(token) = jar.get(COOKIE_PATH) {
        // Validate the token
        return match users.get_role_from_token(token.value()) {
            Ok(UserRole::ReadOnly) => LoginResult::ReadOnly,
            Ok(UserRole::Admin) => LoginResult::Admin,
            Err(_e) => LoginResult::Denied,
        };
    }
    LoginResult::Denied
}

/// Checks an incoming request for a User-Token cookie. If found,
/// it validates the request against the web users file. If the
/// web users file isn't found, it redirects to 'first run'. If
/// it is found, the token is checked (and redirected to login if
/// it isn't good). Finally, the user's role is injected into
/// the middleware.
pub async fn auth_layer(
    jar: CookieJar,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, Html<&'static str>> {
    const BOUNCE: &str =
        "<html><body><script>window.location.href = 'login.html';</script></body></html>";
    const FIRST_RUN: &str =
        "<html><body><script>window.location.href = 'first-run.html';</script></body></html>";

    let mut lock = WEB_USERS.lock().await;
    if lock.is_none() {
        // No lock - let's see if there's a file to use?
        if WebUsers::does_users_file_exist().expect("Error checking if the users file exists") {
            // It exists - we load it
            let users = WebUsers::load_or_create().expect("Unable to load users file");
            *lock = Some(users);
        } else {
            // No users file - redirect to first run
            return Err(Html(FIRST_RUN));
        }
    }

    if let Some(users) = &*lock {
        let login_result = check_login(&jar, users).await;
        return match login_result {
            LoginResult::Admin | LoginResult::ReadOnly => {
                record_first_login_timestamp_if_needed();
                req.extensions_mut().insert(login_result);
                Ok(next.run(req).await)
            }
            LoginResult::Denied => {
                let users = WebUsers::load_or_create().expect("Could not load users file");
                if users.do_we_allow_anonymous() {
                    req.extensions_mut().insert(LoginResult::ReadOnly);
                    Ok(next.run(req).await)
                } else {
                    Err(Html(BOUNCE))
                }
            }
        };
    }
    Err(Html(BOUNCE))
}

pub async fn login_from_token(token: &str) -> LoginResult {
    let mut lock = WEB_USERS.lock().await;
    if lock.is_none() {
        match WebUsers::does_users_file_exist() {
            Ok(true) => {
                match WebUsers::load_or_create() {
                    Ok(users) => {
                        *lock = Some(users);
                    }
                    Err(e) => {
                        warn!("Unable to load users file for websocket auth: {e}");
                        return LoginResult::Denied;
                    }
                }
            }
            Ok(false) => {
                return LoginResult::Denied;
            }
            Err(e) => {
                warn!("Unable to check users file for websocket auth: {e}");
                return LoginResult::Denied;
            }
        }
    }

    let Some(users) = &*lock else {
        return LoginResult::Denied;
    };

    let login_result = match users.get_role_from_token(token) {
        Ok(UserRole::ReadOnly) => LoginResult::ReadOnly,
        Ok(UserRole::Admin) => LoginResult::Admin,
        Err(_) => LoginResult::Denied,
    };

    if login_result != LoginResult::Denied {
        record_first_login_timestamp_if_needed();
    }

    login_result
}

#[derive(Serialize, Deserialize)]
pub struct LoginAttempt {
    pub username: String,
    pub password: String,
}

pub async fn try_login(
    jar: CookieJar,
    Json(login): Json<LoginAttempt>,
) -> Result<(CookieJar, StatusCode), StatusCode> {
    let users = WEB_USERS.lock().await;
    if let Some(users) = &*users {
        return match users.login(&login.username, &login.password) {
            Ok(token) => {
                record_first_login_timestamp_if_needed();
                Ok((jar.add(Cookie::new(COOKIE_PATH, token)), StatusCode::OK))
            }
            Err(..) => {
                if users.do_we_allow_anonymous() {
                    record_first_login_timestamp_if_needed();
                    Ok((jar, StatusCode::OK))
                } else {
                    Err(StatusCode::UNAUTHORIZED)
                }
            }
        };
    }
    Err(StatusCode::UNAUTHORIZED)
}

#[derive(Serialize, Deserialize)]
pub struct FirstUser {
    username: String,
    password: String,
    allow_anonymous: bool,
}

pub async fn first_user(
    jar: CookieJar,
    Json(new_user): Json<FirstUser>,
) -> (CookieJar, StatusCode) {
    let mut users = WebUsers::load_or_create().expect("Could not load users file");
    users
        .allow_anonymous(new_user.allow_anonymous)
        .expect("Unable to set property");
    let token = users
        .add_or_update_user(&new_user.username, &new_user.password, UserRole::Admin)
        .expect("Unable to add or update user");
    let mut lock = WEB_USERS.lock().await;
    *lock = Some(users);
    record_first_login_timestamp_if_needed();
    (jar.add(Cookie::new(COOKIE_PATH, token)), StatusCode::OK)
}
