//! Provides authentication for the Node Manager.
//! This is designed to be broadly compatible with the original
//! cookie-based system, but now uses an Axum layer to be largely
//! invisible.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::Cookie;
use lqos_config::{UserRole, WebUsers};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info};

const COOKIE_PATH: &str = "User-Token";

static WEB_USERS: Lazy<Mutex<Option<WebUsers>>> = Lazy::new(|| Mutex::new(None));

pub async fn invalidate_user_cache() {
    info!("Invalidating user cache");
    let mut lock = WEB_USERS.lock().await;
    *lock = None;
}

pub fn invalidate_user_cache_blocking() {
    info!("Invalidating user cache");
    let mut lock = WEB_USERS.blocking_lock();
    *lock = None;
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
        if WebUsers::does_users_file_exist().unwrap() {
            // It exists - we load it
            let users = WebUsers::load_or_create().unwrap();
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
                req.extensions_mut().insert(login_result);
                Ok(next.run(req).await)
            }
            LoginResult::Denied => {
                let users = WebUsers::load_or_create().unwrap();
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

#[derive(Serialize, Deserialize, Debug)]
pub struct LoginAttempt {
    pub username: String,
    pub password: String,
}

pub async fn try_login(
    jar: CookieJar,
    Json(login): Json<LoginAttempt>,
) -> Result<(CookieJar, StatusCode), StatusCode> {
    debug!("Attempting login for {:?}", login);
    let mut users = WEB_USERS.lock().await;

    if users.is_none() {
        debug!("No users file loaded - attempting to load");
        // No lock - let's see if there's a file to use?
        if WebUsers::does_users_file_exist().unwrap() {
            // It exists - we load it
            let new_users = WebUsers::load_or_create().unwrap();
            *users = Some(new_users);
        }
    }

    if let Some(users) = &*users {
        return match users.login(&login.username, &login.password) {
            Ok(token) => Ok((jar.add(Cookie::new(COOKIE_PATH, token)), StatusCode::OK)),
            Err(..) => {
                if users.do_we_allow_anonymous() {
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
    let mut users = WebUsers::load_or_create().unwrap();
    users.allow_anonymous(new_user.allow_anonymous).unwrap();
    let token = users
        .add_or_update_user(&new_user.username, &new_user.password, UserRole::Admin)
        .unwrap();
    let mut lock = WEB_USERS.lock().await;
    *lock = Some(users);
    (jar.add(Cookie::new(COOKIE_PATH, token)), StatusCode::OK)
}
