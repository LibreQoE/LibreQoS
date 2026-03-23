//! Provides authentication for the Node Manager.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use lqos_config::{AuthenticatedUser, UserRole, WebUsers, load_config};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, warn};

const COOKIE_NAME: &str = "User-Token";
const SESSION_TOKEN_VERSION: &str = "v1";
const SESSION_DURATION_SECS: u64 = 60 * 60 * 24 * 30;
const SESSION_KEY_FILE_NAME: &str = "lqusers.session.key";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthFileFingerprint {
    path: PathBuf,
    modified_unix_secs: Option<u64>,
    modified_subsec_nanos: Option<u32>,
    len: u64,
}

#[derive(Clone, Debug)]
struct CachedAuthSnapshot {
    fingerprint: Option<AuthFileFingerprint>,
    snapshot: AuthSnapshot,
}

#[derive(Clone, Debug)]
struct AuthSnapshot {
    bootstrap_state: AuthBootstrapState,
    auth_epoch: u64,
    allow_anonymous: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthBootstrapState {
    MissingUsersFile,
    NoUsersConfigured,
    Ready,
    CorruptUsersFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionClaims {
    sub: String,
    role: UserRole,
    auth_epoch: u64,
    iat: u64,
    exp: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LoginResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Clone, Debug)]
struct SessionUser {
    username: String,
    role: UserRole,
}

static AUTH_SNAPSHOT: Lazy<Mutex<Option<CachedAuthSnapshot>>> = Lazy::new(|| Mutex::new(None));
static SESSION_KEY: Lazy<Mutex<Option<Vec<u8>>>> = Lazy::new(|| Mutex::new(None));
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

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn auth_file_fingerprint(path: &Path) -> Result<AuthFileFingerprint, std::io::Error> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok());

    Ok(AuthFileFingerprint {
        path: path.to_path_buf(),
        modified_unix_secs: modified.map(|d| d.as_secs()),
        modified_subsec_nanos: modified.map(|d| d.subsec_nanos()),
        len: metadata.len(),
    })
}

fn auth_snapshot() -> AuthSnapshot {
    let current_fingerprint = match WebUsers::existing_path() {
        Ok(Some(path)) => match auth_file_fingerprint(&path) {
            Ok(fingerprint) => Some(fingerprint),
            Err(e) => {
                warn!("Unable to stat auth file {:?}: {e}", path);
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            warn!("Unable to resolve auth file path: {e}");
            return AuthSnapshot {
                bootstrap_state: AuthBootstrapState::CorruptUsersFile,
                auth_epoch: 0,
                allow_anonymous: false,
            };
        }
    };

    let mut cache = AUTH_SNAPSHOT.lock();
    if let Some(cached) = &*cache
        && cached.fingerprint == current_fingerprint
    {
        return cached.snapshot.clone();
    }

    let snapshot = match current_fingerprint {
        None => AuthSnapshot {
            bootstrap_state: AuthBootstrapState::MissingUsersFile,
            auth_epoch: 0,
            allow_anonymous: false,
        },
        Some(_) => match WebUsers::load_or_create() {
            Ok(users) => AuthSnapshot {
                bootstrap_state: if users.is_empty() {
                    AuthBootstrapState::NoUsersConfigured
                } else {
                    AuthBootstrapState::Ready
                },
                auth_epoch: users.auth_epoch(),
                allow_anonymous: users.do_we_allow_anonymous(),
            },
            Err(e) => {
                warn!("Unable to load auth state: {e}");
                AuthSnapshot {
                    bootstrap_state: AuthBootstrapState::CorruptUsersFile,
                    auth_epoch: 0,
                    allow_anonymous: false,
                }
            }
        },
    };

    let refreshed_fingerprint = match WebUsers::existing_path() {
        Ok(Some(path)) => auth_file_fingerprint(&path).ok(),
        Ok(None) => None,
        Err(e) => {
            warn!("Unable to refresh auth file path after load: {e}");
            None
        }
    };

    *cache = Some(CachedAuthSnapshot {
        fingerprint: refreshed_fingerprint,
        snapshot: snapshot.clone(),
    });
    snapshot
}

fn session_key_path() -> Result<PathBuf, std::io::Error> {
    let config = load_config().map_err(|_| {
        std::io::Error::other("Unable to load /etc/lqos.conf while locating session key")
    })?;
    Ok(Path::new(&config.lqos_directory).join(SESSION_KEY_FILE_NAME))
}

fn session_key() -> Result<Vec<u8>, std::io::Error> {
    let mut cache = SESSION_KEY.lock();
    if let Some(key) = &*cache {
        return Ok(key.clone());
    }

    let path = session_key_path()?;
    let key = if path.exists() {
        let bytes = std::fs::read(&path)?;
        if bytes.is_empty() {
            return Err(std::io::Error::other(format!(
                "Session key file {:?} is empty",
                path
            )));
        }
        bytes
    } else {
        let mut new_key = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut new_key);

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(&new_key)?;
        drop(file);
        new_key
    };

    *cache = Some(key.clone());
    Ok(key)
}

fn build_session_cookie(token: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(COOKIE_NAME, token);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
    cookie
}

fn build_signed_session(key: &[u8], user: &AuthenticatedUser) -> Result<String, StatusCode> {
    let now = now_unix_secs();
    let claims = SessionClaims {
        sub: user.username.clone(),
        role: user.role,
        auth_epoch: user.auth_epoch,
        iat: now,
        exp: now.saturating_add(SESSION_DURATION_SECS),
    };
    let payload = serde_json::to_vec(&claims).map_err(|e| {
        error!("Unable to serialize session claims: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload);

    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        error!("Unable to initialize session signer: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    Ok(format!("{SESSION_TOKEN_VERSION}.{payload_b64}.{signature}"))
}

fn verify_signed_session(
    key: &[u8],
    token: &str,
    snapshot: &AuthSnapshot,
) -> Result<Option<SessionUser>, StatusCode> {
    let Some((version, remainder)) = token.split_once('.') else {
        return Ok(None);
    };
    if version != SESSION_TOKEN_VERSION {
        return Ok(None);
    }
    let Some((payload_b64, signature_b64)) = remainder.rsplit_once('.') else {
        return Ok(None);
    };

    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        error!("Unable to initialize session verifier: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if mac.verify_slice(&signature).is_err() {
        return Ok(None);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let claims: SessionClaims =
        serde_json::from_slice(&payload).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let now = now_unix_secs();
    if claims.exp <= now || claims.auth_epoch != snapshot.auth_epoch {
        return Ok(None);
    }

    Ok(Some(SessionUser {
        username: claims.sub,
        role: claims.role,
    }))
}

fn session_from_cookie(
    jar: &CookieJar,
    snapshot: &AuthSnapshot,
) -> Result<Option<SessionUser>, StatusCode> {
    let Some(token) = jar.get(COOKIE_NAME) else {
        return Ok(None);
    };
    let key = session_key().map_err(|e| {
        error!("Unable to load session key: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    verify_signed_session(&key, token.value(), snapshot)
}

fn standalone_page_path(page: &str) -> Result<PathBuf, StatusCode> {
    let config = load_config().map_err(|e| {
        error!("Unable to load config for standalone page {page}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join(page))
}

fn serve_standalone_page(page: &str) -> Result<Response, StatusCode> {
    let path = standalone_page_path(page)?;
    let body = std::fs::read_to_string(&path).map_err(|e| {
        error!("Unable to read standalone page {:?}: {e}", path);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Html(body).into_response())
}

pub async fn login_page(jar: CookieJar) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            Redirect::temporary("/first-run.html").into_response()
        }
        AuthBootstrapState::Ready => match session_from_cookie(&jar, &snapshot) {
            Ok(Some(_)) => Redirect::temporary("/index.html").into_response(),
            Ok(None) => serve_standalone_page("login.html")
                .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
            Err(status) => (status, "Unable to validate login session").into_response(),
        },
        AuthBootstrapState::CorruptUsersFile => serve_standalone_page("login.html")
            .unwrap_or_else(|status| (status, "Unable to serve login page").into_response()),
    }
}

pub async fn first_run_page() -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            serve_standalone_page("first-run.html")
                .unwrap_or_else(|status| (status, "Unable to serve first-run page").into_response())
        }
        AuthBootstrapState::Ready | AuthBootstrapState::CorruptUsersFile => {
            Redirect::temporary("/index.html").into_response()
        }
    }
}

pub async fn get_username(jar: &CookieJar) -> String {
    let snapshot = auth_snapshot();
    match session_from_cookie(jar, &snapshot) {
        Ok(Some(user)) => user.username,
        Ok(None) | Err(_) => "Anonymous".to_string(),
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum LoginResult {
    Admin,
    ReadOnly,
    Denied,
}

fn login_result_for_session(user: Option<SessionUser>, allow_anonymous: bool) -> LoginResult {
    match user {
        Some(SessionUser {
            role: UserRole::Admin,
            ..
        }) => LoginResult::Admin,
        Some(SessionUser {
            role: UserRole::ReadOnly,
            ..
        }) => LoginResult::ReadOnly,
        None if allow_anonymous => LoginResult::ReadOnly,
        None => LoginResult::Denied,
    }
}

/// Checks an incoming request for a `User-Token` cookie. If found,
/// it validates the request against the signed session and current auth epoch.
/// Missing or empty auth state redirects to first-run; invalid sessions redirect
/// to login unless anonymous read-only is enabled.
pub async fn auth_layer(
    jar: CookieJar,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            return Redirect::temporary("/first-run.html").into_response();
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Redirect::temporary("/login.html").into_response();
        }
        AuthBootstrapState::Ready => {}
    }

    let login_result = match session_from_cookie(&jar, &snapshot) {
        Ok(user) => login_result_for_session(user, snapshot.allow_anonymous),
        Err(status) => return (status, "Unable to validate session").into_response(),
    };

    match login_result {
        LoginResult::Admin | LoginResult::ReadOnly => {
            record_first_login_timestamp_if_needed();
            req.extensions_mut().insert(login_result);
            next.run(req).await
        }
        LoginResult::Denied => Redirect::temporary("/login.html").into_response(),
    }
}

pub async fn login_from_token(token: &str) -> LoginResult {
    let snapshot = auth_snapshot();
    if snapshot.bootstrap_state != AuthBootstrapState::Ready {
        return LoginResult::Denied;
    }

    let key = match session_key() {
        Ok(key) => key,
        Err(e) => {
            warn!("Unable to load session key for websocket auth: {e}");
            return LoginResult::Denied;
        }
    };

    let login_result = match verify_signed_session(&key, token, &snapshot) {
        Ok(user) => login_result_for_session(user, snapshot.allow_anonymous),
        Err(e) => {
            warn!("Unable to verify websocket session token: {e}");
            LoginResult::Denied
        }
    };

    if login_result != LoginResult::Denied {
        record_first_login_timestamp_if_needed();
    }

    login_result
}

/// Invalidate the cached auth snapshot after user-management changes.
pub fn invalidate_auth_cache() {
    let mut lock = AUTH_SNAPSHOT.lock();
    *lock = None;
}

/// Reload the cached auth snapshot after user-management changes.
pub async fn refresh_cached_users() {
    invalidate_auth_cache();
}

#[derive(Serialize, Deserialize)]
pub struct LoginAttempt {
    pub username: String,
    pub password: String,
}

pub async fn try_login(
    jar: CookieJar,
    Json(login): Json<LoginAttempt>,
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginResponse>)> {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("first_run_required"),
                    message: Some("No users are configured yet.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("The auth file is corrupt and must be repaired.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::Ready => {}
    }

    let mut users = WebUsers::load_or_create().map_err(|e| {
        warn!("Unable to load users during login: {e}");
        (
            StatusCode::CONFLICT,
            Json(LoginResponse {
                ok: false,
                reason: Some("auth_corrupt"),
                message: Some("The auth file is corrupt and must be repaired.".to_string()),
            }),
        )
    })?;
    let authenticated = users
        .authenticate(&login.username, &login.password)
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("invalid_credentials"),
                    message: Some("Invalid username or password.".to_string()),
                }),
            )
        })?;

    invalidate_auth_cache();
    let key = session_key().map_err(|e| {
        error!("Unable to load session key during login: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;
    let token = build_signed_session(&key, &authenticated).map_err(|status| {
        (
            status,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;

    record_first_login_timestamp_if_needed();
    Ok((
        jar.add(build_session_cookie(token)),
        Json(LoginResponse {
            ok: true,
            reason: None,
            message: None,
        }),
    ))
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
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginResponse>)> {
    let snapshot = auth_snapshot();
    match snapshot.bootstrap_state {
        AuthBootstrapState::Ready => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("already_configured"),
                    message: Some("Web authentication is already configured.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::CorruptUsersFile => {
            return Err((
                StatusCode::CONFLICT,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("The auth file is corrupt and must be repaired.".to_string()),
                }),
            ));
        }
        AuthBootstrapState::MissingUsersFile | AuthBootstrapState::NoUsersConfigured => {}
    }

    let mut users = WebUsers::load_or_create().map_err(|e| {
        warn!("Unable to load users during first-run setup: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("auth_corrupt"),
                message: Some("Unable to initialize auth storage.".to_string()),
            }),
        )
    })?;
    users
        .allow_anonymous(new_user.allow_anonymous)
        .map_err(|e| {
            warn!("Unable to set anonymous auth policy: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("Unable to update auth settings.".to_string()),
                }),
            )
        })?;
    users
        .add_or_update_user(&new_user.username, &new_user.password, UserRole::Admin)
        .map_err(|e| {
            warn!("Unable to create first user: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(LoginResponse {
                    ok: false,
                    reason: Some("auth_corrupt"),
                    message: Some("Unable to create the first user.".to_string()),
                }),
            )
        })?;

    invalidate_auth_cache();
    let authenticated = AuthenticatedUser {
        username: new_user.username,
        role: UserRole::Admin,
        auth_epoch: users.auth_epoch(),
        password_upgraded: false,
    };
    let key = session_key().map_err(|e| {
        error!("Unable to load session key during first-run setup: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;
    let token = build_signed_session(&key, &authenticated).map_err(|status| {
        (
            status,
            Json(LoginResponse {
                ok: false,
                reason: Some("session_error"),
                message: Some("Unable to create session token.".to_string()),
            }),
        )
    })?;

    record_first_login_timestamp_if_needed();
    Ok((
        jar.add(build_session_cookie(token)),
        Json(LoginResponse {
            ok: true,
            reason: None,
            message: None,
        }),
    ))
}
